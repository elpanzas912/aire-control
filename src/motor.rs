//! Lógica del programa sin ventana: lee el aire, aplica cambios, cumple temporizadores y registra todo.
//! Recibe la hora como parámetro para poder probar los temporizadores sin esperar.
use crate::historial::{Historia, Historial};
use crate::nube::{Aire, ErrorNube, Transporte};
use crate::protocolo::{consulta_estado, fijar_estado, leer_estado, Cambios, Estado};
use crate::tiempo::{duracion, hora};

pub const TRAMO_TEMPORIZADOR: i64 = if cfg!(feature = "minuto") { 60 } else { 30 * 60 };
/// Si la PC estuvo apagada más que esto, prender tarde sorprendería: se avisa en vez de prender.
const TOLERANCIA_ENCENDIDO: i64 = 15 * 60;
const REINTENTO_TEMPORIZADOR: i64 = 30;
const AVISAR_FALLA_DESDE: u32 = 3;
const PAUSA_ENTRE_LOGINS: i64 = 60;

const SONDEO_PANEL_ABIERTO: i64 = 20;
const SONDEO_ENCENDIDO: i64 = 60;
const SONDEO_APAGADO: i64 = 5 * 60;
const SONDEO_CON_ERROR: i64 = 60;

const AJUSTE_APAGAR: &str = "temporizador_apagar";
const AJUSTE_ENCENDER: &str = "temporizador_encender";
const AJUSTE_POTENCIA: &str = "potencia_watts";
const AJUSTE_AIRE: &str = "aire_elegido";
pub const POTENCIA_INICIAL: i64 = 1100;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fase {
    SinCuenta,
    Conectando,
    Conectado,
    /// La cuenta fue rechazada: no se reintenta solo.
    CuentaRechazada,
}

#[derive(Clone, Debug)]
pub struct Aviso {
    pub titulo: String,
    pub texto: String,
}

#[derive(Clone, Debug)]
pub struct Vista {
    pub fase: Fase,
    pub estado: Option<Estado>,
    pub error: Option<String>,
    pub actualizado: i64,
    pub apagar_en: Option<i64>,
    pub encender_en: Option<i64>,
    pub cambios_atendidos: u64,
    pub potencia: i64,
    pub lecturas_totales: u64,
    pub primera_lectura: Option<i64>,
    pub historia: Option<(u8, Historia)>,
    pub aires: Vec<Aire>,
    pub aire_actual: Option<u64>,
}

impl Default for Vista {
    fn default() -> Self {
        Self {
            fase: Fase::SinCuenta,
            estado: None,
            error: None,
            actualizado: 0,
            apagar_en: None,
            encender_en: None,
            cambios_atendidos: 0,
            potencia: POTENCIA_INICIAL,
            lecturas_totales: 0,
            primera_lectura: None,
            historia: None,
            aires: Vec::new(),
            aire_actual: None,
        }
    }
}

pub struct Motor {
    transporte: Option<Box<dyn Transporte>>,
    historial: Historial,
    pub vista: Vista,
    pub avisos: Vec<Aviso>,
    id_mensaje: u8,
    ultimo_login: Option<i64>,
    ultimo_crudo: Vec<u8>,
    panel_abierto: bool,
    proximo_sondeo: i64,
    proximo_reintento: i64,
    fallas_temporizador: u32,
}

impl Motor {
    pub fn nuevo(historial: Historial, ahora: i64) -> Self {
        let mut motor = Self {
            transporte: None,
            vista: Vista {
                apagar_en: historial.ajuste_numero(AJUSTE_APAGAR),
                encender_en: historial.ajuste_numero(AJUSTE_ENCENDER),
                potencia: historial.ajuste_numero(AJUSTE_POTENCIA).unwrap_or(POTENCIA_INICIAL),
                ..Default::default()
            },
            historial,
            avisos: Vec::new(),
            id_mensaje: 0,
            ultimo_login: None,
            ultimo_crudo: Vec::new(),
            panel_abierto: false,
            proximo_sondeo: ahora,
            proximo_reintento: 0,
            fallas_temporizador: 0,
        };
        motor.descartar_encendido_vencido(ahora);
        motor.actualizar_totales();
        motor
    }

    fn descartar_encendido_vencido(&mut self, ahora: i64) {
        if let Some(cuando) = self.vista.encender_en {
            if ahora - cuando > TOLERANCIA_ENCENDIDO {
                self.fijar_temporizador_encender(None);
                self.evento(ahora, &format!("No se prendió a las {}: la PC no estaba disponible", hora(cuando)));
                self.avisar("El aire no se prendió", &format!("El temporizador era para las {} y la PC no estaba disponible.", hora(cuando)));
            }
        }
    }

    fn actualizar_totales(&mut self) {
        let (total, primera) = self.historial.total_lecturas();
        self.vista.lecturas_totales = total;
        self.vista.primera_lectura = primera;
    }

    pub fn hay_temporizadores(&self) -> bool {
        self.vista.apagar_en.is_some() || self.vista.encender_en.is_some()
    }

    fn evento(&self, ts: i64, texto: &str) {
        let _ = self.historial.guardar_evento(ts, texto);
    }

    fn avisar(&mut self, titulo: &str, texto: &str) {
        self.avisos.push(Aviso { titulo: titulo.into(), texto: texto.into() });
    }

    fn fijar_temporizador_apagar(&mut self, cuando: Option<i64>) {
        self.vista.apagar_en = cuando;
        let _ = self.historial.fijar_ajuste(AJUSTE_APAGAR, cuando.map(|c| c.to_string()));
    }

    fn fijar_temporizador_encender(&mut self, cuando: Option<i64>) {
        self.vista.encender_en = cuando;
        let _ = self.historial.fijar_ajuste(AJUSTE_ENCENDER, cuando.map(|c| c.to_string()));
    }

    // Cuenta

    pub fn conectar(&mut self, transporte: Box<dyn Transporte>, ahora: i64) -> Result<(), ErrorNube> {
        self.transporte = Some(transporte);
        self.vista.fase = Fase::Conectando;
        self.vista.error = None;
        self.reintentar_conexion(ahora)
    }

    fn reintentar_conexion(&mut self, ahora: i64) -> Result<(), ErrorNube> {
        match self.iniciar_sesion(ahora) {
            Ok(()) => {
                self.vista.fase = Fase::Conectado;
                self.vista.error = None;
                self.proximo_sondeo = ahora;
                Ok(())
            }
            Err(error) => {
                // Sin internet se sigue intentando; si la cuenta fue rechazada, iniciar_sesion ya la frenó
                if self.vista.fase == Fase::Conectando {
                    self.vista.error = Some(error.mensaje());
                    self.proximo_sondeo = ahora + SONDEO_CON_ERROR;
                }
                Err(error)
            }
        }
    }

    pub fn olvidar_cuenta(&mut self) {
        self.transporte = None;
        self.vista.fase = Fase::SinCuenta;
        self.vista.estado = None;
        self.vista.error = None;
        self.vista.aires.clear();
        self.vista.aire_actual = None;
        let _ = self.historial.fijar_ajuste(AJUSTE_AIRE, None);
        self.fijar_temporizador_apagar(None);
        self.fijar_temporizador_encender(None);
    }

    fn iniciar_sesion(&mut self, ahora: i64) -> Result<(), ErrorNube> {
        self.ultimo_login = Some(ahora);
        let transporte = self.transporte.as_mut().ok_or(ErrorNube::Sesion)?;
        match transporte.iniciar_sesion() {
            Err(error @ (ErrorNube::Cuenta { .. } | ErrorNube::Aires { .. })) => {
                self.vista.fase = Fase::CuentaRechazada;
                self.vista.error = Some(error.mensaje());
                Err(error)
            }
            Ok(()) => {
                self.tomar_aires();
                Ok(())
            }
            otro => otro,
        }
    }

    /// Si la cuenta tiene varios aires, vuelve al último que se eligió.
    fn tomar_aires(&mut self) {
        let Some(transporte) = self.transporte.as_mut() else { return };
        let aires = transporte.aires();
        let elegido = self.historial.ajuste_numero(AJUSTE_AIRE).map(|id| id as u64);
        let actual = elegido
            .filter(|id| aires.iter().any(|a| a.id == *id))
            .or_else(|| aires.first().map(|a| a.id));
        if let Some(id) = actual {
            transporte.usar_aire(id);
        }
        self.vista.aires = aires;
        self.vista.aire_actual = actual;
    }

    /// Pasa al siguiente aire de la cuenta. Los temporizadores eran del anterior, así que se cancelan.
    pub fn siguiente_aire(&mut self, ahora: i64) {
        let total = self.vista.aires.len();
        if total < 2 {
            return;
        }
        let posicion = self.vista.aires.iter().position(|a| Some(a.id) == self.vista.aire_actual).unwrap_or(0);
        let nuevo = self.vista.aires[(posicion + 1) % total].clone();
        self.cancelar_apagado(ahora);
        self.cancelar_encendido(ahora);
        if let Some(transporte) = self.transporte.as_mut() {
            transporte.usar_aire(nuevo.id);
        }
        let _ = self.historial.fijar_ajuste(AJUSTE_AIRE, Some(nuevo.id.to_string()));
        self.vista.aire_actual = Some(nuevo.id);
        self.vista.estado = None;
        self.vista.error = None;
        self.ultimo_crudo.clear();
        self.evento(ahora, &format!("Se cambió al aire {}", nuevo.nombre));
        self.sondear(ahora);
    }

    fn intercambiar(&mut self, trama: &[u8], ahora: i64) -> Result<Vec<u8>, ErrorNube> {
        let primera = self.transporte.as_mut().ok_or(ErrorNube::Sesion)?.enviar(trama);
        match primera {
            Err(ErrorNube::Sesion) => {
                // Sesión vencida: el pedido no llegó al aire, así que repetirlo es seguro. Si se acaba de
                // iniciar sesión es porque otro programa la está tomando: no se compite con él.
                if self.ultimo_login.is_some_and(|t| ahora - t < PAUSA_ENTRE_LOGINS) {
                    return Err(ErrorNube::Sesion);
                }
                self.iniciar_sesion(ahora)?;
                self.transporte.as_mut().ok_or(ErrorNube::Sesion)?.enviar(trama)
            }
            otro => otro,
        }
    }

    fn siguiente_id(&mut self) -> u8 {
        self.id_mensaje = self.id_mensaje.wrapping_add(1);
        self.id_mensaje
    }

    // Lectura y registro

    fn leer_aire(&mut self, ahora: i64) -> Result<Estado, ErrorNube> {
        let consulta = consulta_estado(self.siguiente_id());
        let respuesta = self.intercambiar(&consulta, ahora)?;
        leer_estado(&respuesta).ok_or_else(|| ErrorNube::Formato("no es un estado".into()))
    }

    /// Guarda la lectura y deja escrito qué cambió. Si `origen` es None, el cambio vino de afuera.
    fn registrar(&mut self, nuevo: Estado, origen: Option<&str>, ahora: i64) {
        let anterior = self.vista.estado.clone();
        let cambio_crudo = self.ultimo_crudo != nuevo.crudo;
        let _ = self.historial.guardar_lectura(ahora, &nuevo, cambio_crudo);
        self.ultimo_crudo = nuevo.crudo.clone();

        if let Some(previo) = &anterior {
            let quien = origen.unwrap_or("el control remoto u otra app");
            if previo.encendido != nuevo.encendido {
                let accion = if nuevo.encendido { "Se prendió" } else { "Se apagó" };
                self.evento(ahora, &format!("{accion} desde {quien}"));
            }
            if previo.objetivo != nuevo.objetivo {
                self.evento(ahora, &format!("Objetivo en {}° desde {quien}", nuevo.objetivo));
            }
            if previo.modo != nuevo.modo {
                self.evento(ahora, &format!("Modo {} desde {quien}", nombre_modo(nuevo.modo)));
            }
            if previo.ventilador != nuevo.ventilador {
                self.evento(ahora, &format!("Ventilador {} desde {quien}", nombre_ventilador(nuevo.ventilador)));
            }
            if nuevo.filtro && !previo.filtro {
                self.evento(ahora, "El aire avisa que hay que limpiar el filtro");
                self.avisar("Hay que limpiar el filtro", "El aire avisa que el filtro necesita limpieza.");
            }
            if nuevo.error != previo.error && nuevo.error != 0 {
                let texto = format!("El aire informa el código de error {}", nuevo.error);
                self.evento(ahora, &texto);
                self.avisar("El aire informa un error", &texto);
            }
            // Un cambio externo también manda sobre los temporizadores
            if origen.is_none() {
                if previo.encendido && !nuevo.encendido && self.vista.apagar_en.is_some() {
                    self.fijar_temporizador_apagar(None);
                }
            }
        }
        // Con el aire prendido no queda nada por prender, aunque lo hayan prendido con la app cerrada
        if origen.is_none() && nuevo.encendido {
            self.cancelar_encendido(ahora);
        }
        self.vista.estado = Some(nuevo);
        self.vista.actualizado = ahora;
        self.vista.error = None;
        self.actualizar_totales();
    }

    pub fn sondear(&mut self, ahora: i64) {
        if self.vista.fase != Fase::Conectado {
            return;
        }
        match self.leer_aire(ahora) {
            Ok(estado) => {
                self.registrar(estado, None, ahora);
                self.programar_sondeo(ahora, false);
            }
            Err(error) => self.registrar_falla(error, ahora),
        }
    }

    fn registrar_falla(&mut self, error: ErrorNube, ahora: i64) {
        if self.vista.error.is_none() {
            self.evento(ahora, &format!("Sin datos: {}", error.mensaje()));
        }
        self.vista.error = Some(error.mensaje());
        self.programar_sondeo(ahora, true);
    }

    fn programar_sondeo(&mut self, ahora: i64, con_error: bool) {
        let espera = if self.panel_abierto {
            SONDEO_PANEL_ABIERTO
        } else if con_error {
            SONDEO_CON_ERROR
        } else if self.vista.estado.as_ref().is_some_and(|e| e.encendido) {
            SONDEO_ENCENDIDO
        } else {
            SONDEO_APAGADO
        };
        self.proximo_sondeo = ahora + espera;
    }

    pub fn panel_abierto(&mut self, abierto: bool, ahora: i64) {
        self.panel_abierto = abierto;
        if abierto {
            self.proximo_sondeo = ahora;
        } else {
            self.programar_sondeo(ahora, self.vista.error.is_some());
        }
    }

    // Cambios

    fn aplicar(&mut self, cambios: Cambios, origen: &str, ahora: i64) -> Result<(), ErrorNube> {
        // El cambio reenvía el estado completo: sin leer antes pisaría otros ajustes con datos viejos,
        // así que si la lectura falla no se envía nada
        let actual = self.leer_aire(ahora)?;
        self.registrar(actual.clone(), None, ahora);
        let mut nuevo = actual;
        cambios.aplicar_a(&mut nuevo);
        let orden = fijar_estado(&nuevo, false, self.siguiente_id());
        let respuesta = self.intercambiar(&orden, ahora)?;
        let confirmado = leer_estado(&respuesta).unwrap_or(nuevo);
        self.registrar(confirmado, Some(origen), ahora);
        Ok(())
    }

    pub fn cambiar(&mut self, cambios: Cambios, numero: u64, ahora: i64) {
        if cambios.encendido == Some(false) {
            self.fijar_temporizador_apagar(None);
        }
        if cambios.encendido == Some(true) {
            self.fijar_temporizador_encender(None);
        }
        match self.aplicar(cambios, "la app", ahora) {
            Ok(()) => self.programar_sondeo(ahora, false),
            Err(error) => {
                self.registrar_falla(error, ahora);
                self.proximo_sondeo = ahora + 5;
            }
        }
        self.vista.cambios_atendidos = numero;
    }

    // Temporizadores

    /// Cada toque suma 30 minutos: mantiene encendido y después apaga.
    pub fn sumar_apagado(&mut self, ahora: i64) {
        let base = [Some(ahora), self.vista.encender_en, self.vista.apagar_en]
            .into_iter()
            .flatten()
            .max()
            .unwrap_or(ahora);
        let primero = self.vista.apagar_en.is_none();
        self.fijar_temporizador_apagar(Some(base + TRAMO_TEMPORIZADOR));
        self.fallas_temporizador = 0;
        if primero {
            self.evento(ahora, &format!("Temporizador: apagar a las {}", hora(base + TRAMO_TEMPORIZADOR)));
        }
        let apagado = self.vista.estado.as_ref().is_none_or(|e| !e.encendido);
        if apagado && self.vista.encender_en.is_none() {
            let prender = Cambios { encendido: Some(true), ..Default::default() };
            if let Err(error) = self.aplicar(prender, "el temporizador", ahora) {
                self.registrar_falla(error, ahora);
            }
        }
    }

    /// Cada toque suma 30 minutos a la espera antes de prender. Con el aire prendido no hay nada que esperar:
    /// al terminar no cambiaría nada, y parecería que el temporizador no apagó el aire.
    pub fn sumar_encendido(&mut self, ahora: i64) {
        if self.vista.estado.as_ref().is_some_and(|e| e.encendido) {
            return;
        }
        let base = self.vista.encender_en.unwrap_or(ahora).max(ahora);
        let primero = self.vista.encender_en.is_none();
        self.fijar_temporizador_encender(Some(base + TRAMO_TEMPORIZADOR));
        self.fallas_temporizador = 0;
        if primero {
            self.evento(ahora, &format!("Temporizador: prender a las {}", hora(base + TRAMO_TEMPORIZADOR)));
        }
        // El apagado ya programado se corre para que el tiempo encendido no se pierda
        if let Some(apagar) = self.vista.apagar_en {
            if apagar <= base + TRAMO_TEMPORIZADOR {
                self.fijar_temporizador_apagar(Some(apagar + TRAMO_TEMPORIZADOR));
            }
        }
    }

    pub fn cancelar_apagado(&mut self, ahora: i64) {
        if self.vista.apagar_en.take().is_some() {
            self.fijar_temporizador_apagar(None);
            self.evento(ahora, "Temporizador de apagado cancelado");
        }
    }

    pub fn cancelar_encendido(&mut self, ahora: i64) {
        if self.vista.encender_en.take().is_some() {
            self.fijar_temporizador_encender(None);
            self.evento(ahora, "Temporizador de encendido cancelado");
        }
    }

    fn cumplir_temporizador(&mut self, encender: bool, ahora: i64) {
        let cambios = Cambios { encendido: Some(encender), ..Default::default() };
        match self.aplicar(cambios, "el temporizador", ahora) {
            Ok(()) => {
                if encender {
                    self.fijar_temporizador_encender(None);
                    self.avisar("Se prendió el aire", "El temporizador terminó.");
                } else {
                    self.fijar_temporizador_apagar(None);
                    self.avisar("Se apagó el aire", "El temporizador terminó.");
                }
                self.fallas_temporizador = 0;
                self.programar_sondeo(ahora, false);
            }
            Err(error) => {
                self.fallas_temporizador += 1;
                self.proximo_reintento = ahora + REINTENTO_TEMPORIZADOR;
                if self.fallas_temporizador == AVISAR_FALLA_DESDE {
                    let accion = if encender { "prender" } else { "apagar" };
                    let texto = format!("No se pudo {accion} el aire: {} Se sigue intentando.", error.mensaje());
                    self.evento(ahora, &texto);
                    self.avisar("El temporizador no pudo cumplirse", &texto);
                }
                self.registrar_falla(error, ahora);
            }
        }
    }

    /// Hace lo que corresponda a esta hora. Devuelve true si cambió algo que mostrar.
    pub fn tic(&mut self, ahora: i64) -> bool {
        let mut cambio = false;
        if self.vista.fase == Fase::Conectando && self.transporte.is_some() && ahora >= self.proximo_sondeo {
            let _ = self.reintentar_conexion(ahora);
            cambio = true;
        }
        if self.vista.fase == Fase::Conectado && ahora >= self.proximo_reintento {
            self.descartar_encendido_vencido(ahora);
            if self.vista.encender_en.is_some_and(|c| ahora >= c) {
                self.cumplir_temporizador(true, ahora);
                cambio = true;
            }
            if ahora >= self.proximo_reintento && self.vista.apagar_en.is_some_and(|c| ahora >= c) {
                self.cumplir_temporizador(false, ahora);
                cambio = true;
            }
        }
        if self.vista.fase == Fase::Conectado && ahora >= self.proximo_sondeo {
            self.sondear(ahora);
            cambio = true;
        }
        cambio
    }

    /// Cuántos segundos se puede dormir hasta la próxima tarea.
    pub fn segundos_hasta_tarea(&self, ahora: i64) -> i64 {
        if self.vista.fase == Fase::Conectando && self.transporte.is_some() {
            return (self.proximo_sondeo - ahora).clamp(0, 3600);
        }
        if self.vista.fase != Fase::Conectado {
            return 3600;
        }
        let mut proxima = self.proximo_sondeo;
        for cuando in [self.vista.apagar_en, self.vista.encender_en].into_iter().flatten() {
            proxima = proxima.min(cuando.max(self.proximo_reintento));
        }
        (proxima - ahora).clamp(0, 3600)
    }

    // Datos

    pub fn pedir_historia(&mut self, rango: u8, ahora: i64) {
        let dias = match rango {
            0 => 1,
            1 => 7,
            _ => 30,
        };
        let desde = ahora - dias * 86_400;
        if let Ok(historia) = self.historial.historia(desde, ahora, 96) {
            self.vista.historia = Some((rango, historia));
        }
    }

    pub fn fijar_potencia(&mut self, watts: i64) {
        self.vista.potencia = watts.clamp(100, 10_000);
        let _ = self.historial.fijar_ajuste(AJUSTE_POTENCIA, Some(self.vista.potencia.to_string()));
    }

    pub fn exportar_csv(&self, salida: &mut impl std::io::Write) -> Result<u64, String> {
        self.historial.exportar_csv(salida)
    }

    pub fn resumen_tooltip(&self, ahora: i64) -> String {
        let mut texto = match &self.vista.estado {
            Some(e) if e.encendido => format!("Aire prendido: {} a {}°", nombre_modo(e.modo), e.objetivo),
            Some(_) => "Aire apagado".to_string(),
            None => "Aire del cuarto".to_string(),
        };
        if let Some(interior) = self.vista.estado.as_ref().and_then(|e| e.interior) {
            texto.push_str(&format!("\nAdentro {interior}°"));
        }
        if let Some(apagar) = self.vista.apagar_en {
            texto.push_str(&format!("\nSe apaga en {}", duracion(apagar - ahora)));
        }
        if let Some(encender) = self.vista.encender_en {
            texto.push_str(&format!("\nSe prende en {}", duracion(encender - ahora)));
        }
        texto
    }
}

pub fn nombre_modo(modo: u8) -> &'static str {
    use crate::protocolo::*;
    match modo {
        MODO_FRIO => "frío",
        MODO_CALOR => "calor",
        MODO_AUTO => "automático",
        MODO_SECO => "seco",
        MODO_VENTILAR => "ventilar",
        _ => "otro",
    }
}

pub fn nombre_ventilador(velocidad: u8) -> String {
    use crate::protocolo::*;
    match velocidad {
        VENT_AUTO => "automático".to_string(),
        VENT_BAJA => "bajo".to_string(),
        VENT_MEDIA => "medio".to_string(),
        VENT_ALTA => "alto".to_string(),
        otro => format!("al {otro} %"),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::protocolo::*;
    use std::sync::{Arc, Mutex};

    const T0: i64 = 1_800_000_000;
    const MIN: i64 = 60;

    /// Aire simulado que interpreta las tramas reales. `fallas` hace fallar los próximos pedidos.
    #[derive(Default)]
    struct Compartido {
        estado: Estado,
        pedidos: Vec<u8>,
        logins: u32,
        fallas: Vec<ErrorNube>,
        aires: Vec<Aire>,
        usado: Option<u64>,
    }

    #[derive(Clone)]
    struct AireFalso(Arc<Mutex<Compartido>>);

    impl AireFalso {
        fn nuevo(encendido: bool) -> Self {
            Self(Arc::new(Mutex::new(Compartido {
                estado: Estado {
                    encendido,
                    objetivo: 21.0,
                    modo: MODO_FRIO,
                    ventilador: VENT_AUTO,
                    giro: 0xC,
                    eco: true,
                    interior: Some(23.0),
                    exterior: Some(25.0),
                    pantalla: true,
                    ..Default::default()
                },
                ..Default::default()
            })))
        }

        fn encendido(&self) -> bool {
            self.0.lock().unwrap().estado.encendido
        }

        fn cambios_enviados(&self) -> usize {
            self.0.lock().unwrap().pedidos.iter().filter(|p| **p == 0x40).count()
        }

        fn fallar(&self, errores: &[ErrorNube]) {
            self.0.lock().unwrap().fallas = errores.to_vec();
        }

        fn desde_afuera(&self, encendido: bool) {
            self.0.lock().unwrap().estado.encendido = encendido;
        }
    }

    pub fn trama_de_estado(e: &Estado) -> Vec<u8> {
        let mut p = vec![0u8; 19];
        p[0] = 0xC0;
        p[1] = e.encendido as u8;
        p[2] = ((e.objetivo as u8 - 16) & 0xF) | (e.modo << 5);
        p[3] = e.ventilador;
        p[4] = 0x7F;
        p[5] = 0x7F;
        p[7] = e.giro;
        p[9] = if e.eco { 0x10 } else { 0 };
        p[11] = 96;
        p[12] = 100;
        crate::protocolo::trama_de_respuesta(&p)
    }

    impl Transporte for AireFalso {
        fn iniciar_sesion(&mut self) -> Result<(), ErrorNube> {
            self.0.lock().unwrap().logins += 1;
            Ok(())
        }

        fn enviar(&mut self, trama: &[u8]) -> Result<Vec<u8>, ErrorNube> {
            let mut c = self.0.lock().unwrap();
            c.pedidos.push(trama[10]);
            if !c.fallas.is_empty() {
                return Err(c.fallas.remove(0));
            }
            if trama[10] == 0x40 {
                c.estado.encendido = trama[11] & 1 != 0;
                c.estado.objetivo = (trama[12] & 0xF) as f32 + 16.0;
                c.estado.modo = trama[12] >> 5;
                c.estado.ventilador = trama[13];
                c.estado.giro = trama[17] & 0xF;
                c.estado.eco = trama[19] & 0x80 != 0;
            }
            Ok(trama_de_estado(&c.estado))
        }

        fn aires(&self) -> Vec<Aire> {
            self.0.lock().unwrap().aires.clone()
        }

        fn usar_aire(&mut self, id: u64) {
            self.0.lock().unwrap().usado = Some(id);
        }
    }

    fn con_dos_aires() -> AireFalso {
        let aire = AireFalso::nuevo(true);
        aire.0.lock().unwrap().aires =
            vec![Aire { id: 1, nombre: "Cuarto".into() }, Aire { id: 2, nombre: "Living".into() }];
        aire
    }

    fn preparar(encendido: bool) -> (Motor, AireFalso) {
        let aire = AireFalso::nuevo(encendido);
        let mut motor = Motor::nuevo(Historial::en_memoria().unwrap(), T0);
        motor.conectar(Box::new(aire.clone()), T0).unwrap();
        motor.tic(T0);
        (motor, aire)
    }

    #[test]
    fn tres_toques_son_noventa_minutos() {
        let (mut m, aire) = preparar(false);
        m.sumar_apagado(T0);
        assert!(aire.encendido(), "el primer toque prende");
        m.sumar_apagado(T0 + 5);
        m.sumar_apagado(T0 + 10);
        assert_eq!(m.vista.apagar_en, Some(T0 + 90 * MIN));

        m.tic(T0 + 89 * MIN);
        assert!(aire.encendido(), "todavía no vence");
        m.tic(T0 + 90 * MIN);
        assert!(!aire.encendido(), "a los 90 minutos apaga");
        assert_eq!(m.vista.apagar_en, None);
        assert_eq!(m.avisos.last().unwrap().titulo, "Se apagó el aire");
    }

    #[test]
    fn si_ya_estaba_prendido_no_reenvia_el_encendido() {
        let (mut m, aire) = preparar(true);
        m.sumar_apagado(T0);
        assert_eq!(aire.cambios_enviados(), 0);
        assert_eq!(m.vista.apagar_en, Some(T0 + 30 * MIN));
    }

    #[test]
    fn apagar_a_mano_cancela_el_temporizador() {
        let (mut m, aire) = preparar(true);
        m.sumar_apagado(T0);
        m.cambiar(Cambios { encendido: Some(false), ..Default::default() }, 1, T0 + 60);
        assert_eq!(m.vista.apagar_en, None);
        assert_eq!(m.vista.cambios_atendidos, 1);
        assert!(!aire.encendido());
    }

    #[test]
    fn apagar_con_el_control_remoto_cancela_el_temporizador() {
        let (mut m, aire) = preparar(true);
        m.sumar_apagado(T0);
        aire.desde_afuera(false);
        m.sondear(T0 + 60);
        assert_eq!(m.vista.apagar_en, None);
        let h = m.historial.historia(T0 - 10, T0 + 100, 10).unwrap();
        assert!(h.eventos.iter().any(|e| e.texto.contains("control remoto")));
    }

    #[test]
    fn prender_mas_tarde_suma_de_a_treinta() {
        let (mut m, aire) = preparar(false);
        m.sumar_encendido(T0);
        m.sumar_encendido(T0 + 5);
        assert_eq!(m.vista.encender_en, Some(T0 + 60 * MIN));
        assert!(!aire.encendido());
        m.tic(T0 + 60 * MIN);
        assert!(aire.encendido());
        assert_eq!(m.vista.encender_en, None);
    }

    #[test]
    fn con_el_aire_prendido_no_se_puede_prender_mas_tarde() {
        let (mut m, aire) = preparar(true);
        m.sumar_encendido(T0);
        assert_eq!(m.vista.encender_en, None);
        m.tic(T0 + 31 * MIN);
        assert_eq!(aire.cambios_enviados(), 0);
        assert!(m.avisos.is_empty(), "no avisa que prendió algo que ya estaba prendido");
    }

    #[test]
    fn prender_mas_tarde_queda_en_el_historial() {
        let (mut m, _) = preparar(false);
        m.sumar_encendido(T0);
        m.sumar_encendido(T0 + 5);
        let h = m.historial.historia(T0 - 10, T0 + 100, 10).unwrap();
        assert_eq!(h.eventos.iter().filter(|e| e.texto.starts_with("Temporizador: prender a las")).count(), 1);
    }

    #[test]
    fn si_lo_prendieron_con_la_app_cerrada_no_queda_nada_por_prender() {
        let historial = Historial::en_memoria().unwrap();
        historial.fijar_ajuste(AJUSTE_ENCENDER, Some((T0 + 10 * MIN).to_string())).unwrap();
        let aire = AireFalso::nuevo(true);
        let mut m = Motor::nuevo(historial, T0);
        assert_eq!(m.vista.encender_en, Some(T0 + 10 * MIN));
        m.conectar(Box::new(aire.clone()), T0).unwrap();
        m.tic(T0);
        assert_eq!(m.vista.encender_en, None);
        m.tic(T0 + 11 * MIN);
        assert_eq!(aire.cambios_enviados(), 0);
    }

    #[test]
    fn se_puede_prender_mas_tarde_por_un_rato() {
        let (mut m, aire) = preparar(false);
        m.sumar_encendido(T0);
        m.sumar_apagado(T0 + 1);
        assert!(!aire.encendido(), "no prende ahora: espera al temporizador");
        assert_eq!(m.vista.apagar_en, Some(T0 + 60 * MIN));
        m.tic(T0 + 30 * MIN);
        assert!(aire.encendido());
        m.tic(T0 + 60 * MIN);
        assert!(!aire.encendido());
    }

    #[test]
    fn cancelar_no_toca_el_aire() {
        let (mut m, aire) = preparar(true);
        m.sumar_apagado(T0);
        m.cancelar_apagado(T0 + 1);
        m.tic(T0 + 31 * MIN);
        assert!(aire.encendido());
        assert_eq!(aire.cambios_enviados(), 0);
    }

    #[test]
    fn reintenta_si_el_temporizador_falla_y_avisa_una_vez() {
        let (mut m, aire) = preparar(true);
        m.sumar_apagado(T0);
        let falla = ErrorNube::Servidor { codigo: "3176".into() };
        aire.fallar(&[falla.clone(), falla.clone(), falla]);
        let vence = T0 + 30 * MIN;
        m.tic(vence);
        assert!(aire.encendido());
        assert_eq!(m.vista.apagar_en, Some(vence - 0), "sigue pendiente");
        m.tic(vence + 10);
        assert_eq!(aire.0.lock().unwrap().pedidos.iter().filter(|p| **p == 0x41).count(), 2, "respeta la pausa");
        m.tic(vence + 30);
        m.tic(vence + 60);
        assert_eq!(m.avisos.iter().filter(|a| a.titulo.contains("no pudo")).count(), 1);
        m.tic(vence + 90);
        assert!(!aire.encendido(), "cuando vuelve a responder, cumple");
        assert_eq!(m.vista.apagar_en, None);
    }

    #[test]
    fn no_envia_cambios_si_no_pudo_leer() {
        let (mut m, aire) = preparar(false);
        aire.fallar(&[ErrorNube::Servidor { codigo: "3176".into() }]);
        m.cambiar(Cambios { encendido: Some(true), ..Default::default() }, 7, T0 + 1);
        assert!(!aire.encendido());
        assert_eq!(aire.cambios_enviados(), 0);
        assert!(m.vista.error.as_ref().unwrap().contains("no respondió"));
        assert_eq!(m.vista.cambios_atendidos, 7, "la ventana deja de esperar");
    }

    #[test]
    fn un_cambio_conserva_los_demas_ajustes() {
        let (mut m, aire) = preparar(false);
        m.cambiar(Cambios { objetivo: Some(24.0), ..Default::default() }, 1, T0 + 1);
        let c = aire.0.lock().unwrap();
        assert_eq!(c.estado.objetivo, 24.0);
        assert_eq!(c.estado.giro, 0xC);
        assert!(c.estado.eco);
        assert!(!c.estado.encendido);
    }

    #[test]
    fn renueva_la_sesion_una_vez() {
        let (mut m, aire) = preparar(false);
        aire.fallar(&[ErrorNube::Sesion]);
        m.sondear(T0 + 2 * MIN);
        assert_eq!(aire.0.lock().unwrap().logins, 2);
        assert!(m.vista.error.is_none());
    }

    #[test]
    fn no_compite_por_la_sesion_con_otro_programa() {
        let (mut m, aire) = preparar(false);
        aire.fallar(&[ErrorNube::Sesion]);
        m.sondear(T0 + 5);
        assert_eq!(aire.0.lock().unwrap().logins, 1, "recién inició sesión: no reintenta");
        assert!(m.vista.error.as_ref().unwrap().contains("Otro programa"));
    }

    #[test]
    fn la_cuenta_rechazada_no_se_reintenta() {
        struct Rechaza(Arc<Mutex<u32>>);
        impl Transporte for Rechaza {
            fn iniciar_sesion(&mut self) -> Result<(), ErrorNube> {
                *self.0.lock().unwrap() += 1;
                Err(ErrorNube::Cuenta { codigo: "3102".into() })
            }
            fn enviar(&mut self, _: &[u8]) -> Result<Vec<u8>, ErrorNube> {
                Err(ErrorNube::Sesion)
            }
        }
        let intentos = Arc::new(Mutex::new(0));
        let mut m = Motor::nuevo(Historial::en_memoria().unwrap(), T0);
        assert!(m.conectar(Box::new(Rechaza(intentos.clone())), T0).is_err());
        assert_eq!(m.vista.fase, Fase::CuentaRechazada);
        m.tic(T0 + 3600);
        assert_eq!(*intentos.lock().unwrap(), 1);
    }

    #[test]
    fn sin_internet_sigue_intentando_conectar() {
        struct SinRed(Arc<Mutex<u32>>);
        impl Transporte for SinRed {
            fn iniciar_sesion(&mut self) -> Result<(), ErrorNube> {
                let mut n = self.0.lock().unwrap();
                *n += 1;
                if *n < 3 {
                    Err(ErrorNube::Red("sin red".into()))
                } else {
                    Ok(())
                }
            }
            fn enviar(&mut self, _: &[u8]) -> Result<Vec<u8>, ErrorNube> {
                Err(ErrorNube::Red("sin red".into()))
            }
        }
        let intentos = Arc::new(Mutex::new(0));
        let mut m = Motor::nuevo(Historial::en_memoria().unwrap(), T0);
        assert!(m.conectar(Box::new(SinRed(intentos.clone())), T0).is_err());
        assert_eq!(m.vista.fase, Fase::Conectando);
        assert!(m.vista.error.as_ref().unwrap().contains("internet"));
        assert_eq!(m.segundos_hasta_tarea(T0), SONDEO_CON_ERROR);
        m.tic(T0 + 10);
        assert_eq!(*intentos.lock().unwrap(), 1, "respeta la pausa");
        m.tic(T0 + SONDEO_CON_ERROR);
        assert_eq!(*intentos.lock().unwrap(), 2);
        m.tic(T0 + 2 * SONDEO_CON_ERROR);
        assert_eq!(m.vista.fase, Fase::Conectado);
        assert!(m.vista.error.is_none() || m.vista.error.as_ref().unwrap().contains("internet"));
    }

    #[test]
    fn cuenta_sin_aires_no_se_reintenta_sola() {
        struct SinAires(Arc<Mutex<u32>>);
        impl Transporte for SinAires {
            fn iniciar_sesion(&mut self) -> Result<(), ErrorNube> {
                *self.0.lock().unwrap() += 1;
                Err(ErrorNube::Aires { codigo: None })
            }
            fn enviar(&mut self, _: &[u8]) -> Result<Vec<u8>, ErrorNube> {
                Err(ErrorNube::Sesion)
            }
        }
        let intentos = Arc::new(Mutex::new(0));
        let mut m = Motor::nuevo(Historial::en_memoria().unwrap(), T0);
        assert!(m.conectar(Box::new(SinAires(intentos.clone())), T0).is_err());
        assert_eq!(m.vista.fase, Fase::CuentaRechazada);
        assert!(m.vista.error.as_ref().unwrap().contains("ningún aire"));
        m.tic(T0 + 3600);
        assert_eq!(*intentos.lock().unwrap(), 1);
    }

    #[test]
    fn cambiar_de_aire_cancela_los_temporizadores_y_lee_el_nuevo() {
        let aire = con_dos_aires();
        let mut m = Motor::nuevo(Historial::en_memoria().unwrap(), T0);
        m.conectar(Box::new(aire.clone()), T0).unwrap();
        assert_eq!(m.vista.aire_actual, Some(1));
        m.tic(T0);
        m.sumar_apagado(T0);

        m.siguiente_aire(T0 + 1);
        assert_eq!(aire.0.lock().unwrap().usado, Some(2));
        assert_eq!(m.vista.aire_actual, Some(2));
        assert_eq!(m.vista.apagar_en, None);
        assert!(m.vista.estado.is_some());

        m.siguiente_aire(T0 + 2);
        assert_eq!(m.vista.aire_actual, Some(1), "da la vuelta");
    }

    #[test]
    fn con_un_solo_aire_no_hay_a_cual_cambiar() {
        let aire = AireFalso::nuevo(true);
        aire.0.lock().unwrap().aires = vec![Aire { id: 7, nombre: "Cuarto".into() }];
        let mut m = Motor::nuevo(Historial::en_memoria().unwrap(), T0);
        m.conectar(Box::new(aire.clone()), T0).unwrap();
        m.sumar_apagado(T0);
        m.siguiente_aire(T0 + 1);
        assert_eq!(m.vista.aire_actual, Some(7));
        assert!(m.vista.apagar_en.is_some());
    }

    #[test]
    fn vuelve_al_aire_elegido_si_la_cuenta_todavia_lo_tiene() {
        let mut m = Motor::nuevo(Historial::en_memoria().unwrap(), T0);
        m.conectar(Box::new(con_dos_aires()), T0).unwrap();
        m.siguiente_aire(T0);

        let misma_cuenta = con_dos_aires();
        m.conectar(Box::new(misma_cuenta.clone()), T0 + 60).unwrap();
        assert_eq!(misma_cuenta.0.lock().unwrap().usado, Some(2));

        let otra_cuenta = AireFalso::nuevo(true);
        otra_cuenta.0.lock().unwrap().aires = vec![Aire { id: 7, nombre: "Otro".into() }];
        m.conectar(Box::new(otra_cuenta.clone()), T0 + 120).unwrap();
        assert_eq!(otra_cuenta.0.lock().unwrap().usado, Some(7));

        m.olvidar_cuenta();
        assert!(m.vista.aires.is_empty());
        assert_eq!(m.vista.aire_actual, None);
    }

    #[test]
    fn los_temporizadores_sobreviven_a_un_reinicio() {
        let ruta = std::env::temp_dir().join(format!("aire-prueba-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&ruta);
        {
            let aire = AireFalso::nuevo(true);
            let mut m = Motor::nuevo(Historial::abrir(&ruta).unwrap(), T0);
            m.conectar(Box::new(aire), T0).unwrap();
            m.sumar_apagado(T0);
        }
        let m = Motor::nuevo(Historial::abrir(&ruta).unwrap(), T0 + 60);
        assert_eq!(m.vista.apagar_en, Some(T0 + 30 * MIN));
        drop(m);
        for sufijo in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{sufijo}", ruta.display()));
        }
    }

    #[test]
    fn apagado_vencido_se_cumple_pero_encendido_vencido_se_avisa() {
        let ruta = std::env::temp_dir().join(format!("aire-prueba-v-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&ruta);
        {
            let h = Historial::abrir(&ruta).unwrap();
            h.fijar_ajuste(AJUSTE_APAGAR, Some((T0 - 7200).to_string())).unwrap();
            h.fijar_ajuste(AJUSTE_ENCENDER, Some((T0 - 7200).to_string())).unwrap();
        }
        let aire = AireFalso::nuevo(true);
        let mut m = Motor::nuevo(Historial::abrir(&ruta).unwrap(), T0);
        assert_eq!(m.vista.encender_en, None, "no prende por sorpresa");
        assert!(m.avisos[0].titulo.contains("no se prendió"));
        m.conectar(Box::new(aire.clone()), T0).unwrap();
        m.tic(T0);
        assert!(!aire.encendido(), "el apagado vencido sí se cumple");
        drop(m);
        for sufijo in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{sufijo}", ruta.display()));
        }
    }

    #[test]
    fn sondea_menos_cuando_el_aire_esta_apagado() {
        let (mut m, _) = preparar(false);
        assert_eq!(m.segundos_hasta_tarea(T0), SONDEO_APAGADO);
        m.panel_abierto(true, T0);
        assert_eq!(m.segundos_hasta_tarea(T0), 0);
        m.tic(T0);
        assert_eq!(m.segundos_hasta_tarea(T0), SONDEO_PANEL_ABIERTO);
        m.panel_abierto(false, T0);
        assert_eq!(m.segundos_hasta_tarea(T0), SONDEO_APAGADO);
    }

    #[test]
    fn registra_cada_cambio_como_evento() {
        let (mut m, _) = preparar(false);
        m.cambiar(Cambios { encendido: Some(true), objetivo: Some(24.0), ..Default::default() }, 1, T0 + 10);
        let h = m.historial.historia(T0 - 10, T0 + 100, 10).unwrap();
        let textos: Vec<&str> = h.eventos.iter().map(|e| e.texto.as_str()).collect();
        assert!(textos.contains(&"Se prendió desde la app"));
        assert!(textos.contains(&"Objetivo en 24° desde la app"));
    }
}
