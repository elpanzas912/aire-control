//! Tramas del aire: consultas, cambios y lectura del estado. Es un puerto de lo que hace msmart-ng.
use crate::cifrado::{descifrar_ecb, md5};

const TIPO_AIRE: u8 = 0xAC;
const TRAMA_CONTROL: u8 = 0x02;
const TRAMA_CONSULTA: u8 = 0x03;
const RESPUESTA_ESTADO: u8 = 0xC0;
const FUENTE_APP: u8 = 0x02;
const CLAVE_FIRMA: &[u8] = b"xhdiwjnchekd4d512chdjx5d8e4c394D2D7S";

pub const MODO_AUTO: u8 = 1;
pub const MODO_FRIO: u8 = 2;
pub const MODO_SECO: u8 = 3;
pub const MODO_CALOR: u8 = 4;
pub const MODO_VENTILAR: u8 = 5;

pub const VENT_AUTO: u8 = 102;
pub const VENT_ALTA: u8 = 80;
pub const VENT_MEDIA: u8 = 60;
pub const VENT_BAJA: u8 = 40;

pub const TEMP_MINIMA: f32 = 16.0;
pub const TEMP_MAXIMA: f32 = 30.0;

#[rustfmt::skip]
const TABLA_CRC8: [u8; 256] = [
    0x00, 0x5E, 0xBC, 0xE2, 0x61, 0x3F, 0xDD, 0x83, 0xC2, 0x9C, 0x7E, 0x20, 0xA3, 0xFD, 0x1F, 0x41,
    0x9D, 0xC3, 0x21, 0x7F, 0xFC, 0xA2, 0x40, 0x1E, 0x5F, 0x01, 0xE3, 0xBD, 0x3E, 0x60, 0x82, 0xDC,
    0x23, 0x7D, 0x9F, 0xC1, 0x42, 0x1C, 0xFE, 0xA0, 0xE1, 0xBF, 0x5D, 0x03, 0x80, 0xDE, 0x3C, 0x62,
    0xBE, 0xE0, 0x02, 0x5C, 0xDF, 0x81, 0x63, 0x3D, 0x7C, 0x22, 0xC0, 0x9E, 0x1D, 0x43, 0xA1, 0xFF,
    0x46, 0x18, 0xFA, 0xA4, 0x27, 0x79, 0x9B, 0xC5, 0x84, 0xDA, 0x38, 0x66, 0xE5, 0xBB, 0x59, 0x07,
    0xDB, 0x85, 0x67, 0x39, 0xBA, 0xE4, 0x06, 0x58, 0x19, 0x47, 0xA5, 0xFB, 0x78, 0x26, 0xC4, 0x9A,
    0x65, 0x3B, 0xD9, 0x87, 0x04, 0x5A, 0xB8, 0xE6, 0xA7, 0xF9, 0x1B, 0x45, 0xC6, 0x98, 0x7A, 0x24,
    0xF8, 0xA6, 0x44, 0x1A, 0x99, 0xC7, 0x25, 0x7B, 0x3A, 0x64, 0x86, 0xD8, 0x5B, 0x05, 0xE7, 0xB9,
    0x8C, 0xD2, 0x30, 0x6E, 0xED, 0xB3, 0x51, 0x0F, 0x4E, 0x10, 0xF2, 0xAC, 0x2F, 0x71, 0x93, 0xCD,
    0x11, 0x4F, 0xAD, 0xF3, 0x70, 0x2E, 0xCC, 0x92, 0xD3, 0x8D, 0x6F, 0x31, 0xB2, 0xEC, 0x0E, 0x50,
    0xAF, 0xF1, 0x13, 0x4D, 0xCE, 0x90, 0x72, 0x2C, 0x6D, 0x33, 0xD1, 0x8F, 0x0C, 0x52, 0xB0, 0xEE,
    0x32, 0x6C, 0x8E, 0xD0, 0x53, 0x0D, 0xEF, 0xB1, 0xF0, 0xAE, 0x4C, 0x12, 0x91, 0xCF, 0x2D, 0x73,
    0xCA, 0x94, 0x76, 0x28, 0xAB, 0xF5, 0x17, 0x49, 0x08, 0x56, 0xB4, 0xEA, 0x69, 0x37, 0xD5, 0x8B,
    0x57, 0x09, 0xEB, 0xB5, 0x36, 0x68, 0x8A, 0xD4, 0x95, 0xCB, 0x29, 0x77, 0xF4, 0xAA, 0x48, 0x16,
    0xE9, 0xB7, 0x55, 0x0B, 0x88, 0xD6, 0x34, 0x6A, 0x2B, 0x75, 0x97, 0xC9, 0x4A, 0x14, 0xF6, 0xA8,
    0x74, 0x2A, 0xC8, 0x96, 0x15, 0x4B, 0xA9, 0xF7, 0xB6, 0xE8, 0x0A, 0x54, 0xD7, 0x89, 0x6B, 0x35,
];

fn crc8(datos: &[u8]) -> u8 {
    datos.iter().fold(0, |crc, b| TABLA_CRC8[(crc ^ b) as usize])
}

fn suma_control(datos: &[u8]) -> u8 {
    datos.iter().fold(0u8, |s, b| s.wrapping_add(*b)).wrapping_neg()
}

fn trama(tipo: u8, datos: &[u8]) -> Vec<u8> {
    let mut salida = vec![0u8; 10];
    salida[0] = 0xAA;
    salida[1] = (datos.len() + 10) as u8;
    salida[2] = TIPO_AIRE;
    salida[9] = tipo;
    salida.extend_from_slice(datos);
    salida.push(suma_control(&salida[1..]));
    salida
}

fn comando(tipo: u8, datos: &[u8], id_mensaje: u8) -> Vec<u8> {
    let mut cuerpo = datos.to_vec();
    cuerpo.push(id_mensaje);
    cuerpo.push(crc8(&cuerpo));
    trama(tipo, &cuerpo)
}

pub fn consulta_estado(id_mensaje: u8) -> Vec<u8> {
    #[rustfmt::skip]
    let datos = [
        0x41, 0x81, 0x00, 0xFF, 0x03, 0xFF, 0x00,
        0x02, // pide la temperatura interior
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0x03,
    ];
    comando(TRAMA_CONSULTA, &datos, id_mensaje)
}

/// Todo lo que el aire informa en su respuesta de estado.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Estado {
    pub encendido: bool,
    pub objetivo: f32,
    pub modo: u8,
    pub ventilador: u8,
    pub giro: u8,
    pub turbo: bool,
    pub eco: bool,
    pub sueno: bool,
    pub fahrenheit: bool,
    pub seguir: bool,
    pub purificador: bool,
    pub aux: bool,
    pub aux_solo: bool,
    pub interior: Option<f32>,
    pub exterior: Option<f32>,
    pub filtro: bool,
    pub pantalla: bool,
    pub error: u8,
    pub humedad_objetivo: Option<u8>,
    pub proteccion_helada: bool,
    pub crudo: Vec<u8>,
}

/// Lo que se puede pedir desde el panel. Lo demás se conserva tal como lo informa el aire.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cambios {
    pub encendido: Option<bool>,
    pub modo: Option<u8>,
    pub objetivo: Option<f32>,
    pub ventilador: Option<u8>,
}

impl Cambios {
    pub fn vacio(&self) -> bool {
        *self == Self::default()
    }

    pub fn sumar(&mut self, otros: Cambios) {
        self.encendido = otros.encendido.or(self.encendido);
        self.modo = otros.modo.or(self.modo);
        self.objetivo = otros.objetivo.or(self.objetivo);
        self.ventilador = otros.ventilador.or(self.ventilador);
    }

    pub fn aplicar_a(&self, estado: &mut Estado) {
        if let Some(v) = self.encendido {
            estado.encendido = v;
        }
        if let Some(v) = self.modo {
            estado.modo = v;
        }
        if let Some(v) = self.objetivo {
            estado.objetivo = v.clamp(TEMP_MINIMA, TEMP_MAXIMA);
        }
        if let Some(v) = self.ventilador {
            estado.ventilador = v;
        }
    }
}

pub fn fijar_estado(e: &Estado, beep: bool, id_mensaje: u8) -> Vec<u8> {
    let entero = e.objetivo.trunc() as i32;
    let (mut temperatura, temperatura_alt) = if (17..=30).contains(&entero) {
        (((entero - 16) & 0xF) as u8, 0u8)
    } else {
        (0u8, ((entero - 12) & 0x1F) as u8)
    };
    if e.objetivo.fract() > 0.0 {
        temperatura |= 0x10;
    }
    let bit = |activo: bool, valor: u8| if activo { valor } else { 0 };
    let datos = [
        0x40,
        FUENTE_APP | bit(beep, 0x40) | bit(e.encendido, 0x01),
        temperatura | ((e.modo & 0x7) << 5),
        e.ventilador,
        0x7F,
        0x7F,
        0x00,
        0x30 | (e.giro & 0x3F),
        bit(e.seguir, 0x80) | bit(e.turbo, 0x20),
        bit(e.eco, 0x80) | bit(e.purificador, 0x20) | bit(e.aux, 0x08),
        bit(e.sueno, 0x01) | bit(e.turbo, 0x02) | bit(e.fahrenheit, 0x04),
        0, 0, 0, 0, 0, 0, 0,
        temperatura_alt,
        e.humedad_objetivo.unwrap_or(40) & 0x7F,
        0,
        bit(e.proteccion_helada, 0x80),
        bit(e.aux_solo, 0x08),
        0,
    ];
    comando(TRAMA_CONTROL, &datos, id_mensaje)
}

fn leer_temperatura(dato: u8, decimales: f32, fahrenheit: bool) -> Option<f32> {
    if dato == 0xFF {
        return None;
    }
    let t = (dato as f32 - 50.0) / 2.0;
    let signo = if t >= 0.0 { 1.0 } else { -1.0 };
    Some(if !fahrenheit && decimales != 0.0 {
        t.trunc() + signo * decimales
    } else if decimales >= 0.5 {
        t.trunc() + signo * 0.5
    } else {
        t
    })
}

/// Interpreta una respuesta completa. `None` si no es un estado válido: nunca se adivina.
pub fn leer_estado(t: &[u8]) -> Option<Estado> {
    if t.len() < 13 || t[0] != 0xAA || t[2] != TIPO_AIRE {
        return None;
    }
    if suma_control(&t[1..t.len() - 1]) != t[t.len() - 1] || t[10] != RESPUESTA_ESTADO {
        return None;
    }
    let tramo = &t[10..t.len() - 1];
    let (datos, control) = tramo.split_at(tramo.len() - 1);
    if crc8(datos) != control[0] && suma_control(datos) != control[0] {
        return None;
    }
    let p = &t[10..t.len() - 2];
    if p.len() < 17 {
        return None;
    }
    let fahrenheit = p[10] & 0x04 != 0;
    let media = if p[2] & 0x10 != 0 { 0.5 } else { 0.0 };
    let alternativa = p[13] & 0x1F;
    let objetivo = if alternativa != 0 {
        alternativa as f32 + 12.0 + media
    } else {
        (p[2] & 0xF) as f32 + 16.0 + media
    };
    Some(Estado {
        encendido: p[1] & 0x01 != 0,
        objetivo,
        modo: (p[2] >> 5) & 0x7,
        ventilador: p[3] & 0x7F,
        giro: p[7] & 0xF,
        turbo: p[8] & 0x20 != 0 || p[10] & 0x02 != 0,
        eco: p[9] & 0x10 != 0,
        sueno: p[10] & 0x01 != 0,
        fahrenheit,
        seguir: p[8] & 0x80 != 0,
        purificador: p[9] & 0x20 != 0,
        aux: p[9] & 0x08 != 0,
        aux_solo: p[8] & 0x40 != 0,
        interior: leer_temperatura(p[11], (p[15] & 0xF) as f32 / 10.0, fahrenheit),
        exterior: leer_temperatura(p[12], (p[15] >> 4) as f32 / 10.0, fahrenheit),
        filtro: p[13] & 0x20 != 0,
        pantalla: p[14] != 0x70,
        error: p[16],
        humedad_objetivo: p.get(19).map(|v| v & 0x7F),
        proteccion_helada: p.get(21).is_some_and(|v| v & 0x80 != 0),
        crudo: p.to_vec(),
    })
}

/// Arma una respuesta válida: la usan el aire simulado y las pruebas.
#[cfg(any(test, feature = "demo"))]
pub fn trama_de_respuesta(datos: &[u8]) -> Vec<u8> {
    let mut cuerpo = datos.to_vec();
    cuerpo.push(crc8(datos));
    trama(TRAMA_CONSULTA, &cuerpo)
}

pub fn firmar(datos: &[u8]) -> [u8; 16] {
    let mut todo = datos.to_vec();
    todo.extend_from_slice(CLAVE_FIRMA);
    md5(&todo)
}

/// Paquete 0x5A5A con la trama sin cifrar, como lo arma la app para la nube.
pub fn armar_paquete(aire_id: u64, datos: &[u8], marca: [u8; 8]) -> Vec<u8> {
    let largo = 40 + datos.len() + 16;
    let mut paquete = vec![0x5A, 0x5A, 0x01, 0x10];
    paquete.extend_from_slice(&(largo as u16).to_le_bytes());
    paquete.extend_from_slice(&[0x20, 0x00, 0x50, 0x00, 0x00, 0x00]);
    paquete.extend_from_slice(&marca);
    paquete.extend_from_slice(&aire_id.to_le_bytes());
    paquete.extend_from_slice(&[0xB6, 0x05, 0x02, 0x04, 0x05, 0x04, 0, 0, 0, 0, 0, 0]);
    paquete.extend_from_slice(datos);
    let firma = firmar(&paquete);
    paquete.extend_from_slice(&firma);
    paquete
}

/// Deja solo la trama 0xAA, venga como venga la respuesta de la nube.
pub fn extraer_trama(respuesta: &[u8]) -> Option<Vec<u8>> {
    match respuesta.first()? {
        0xAA => Some(respuesta.to_vec()),
        0x5A if respuesta.get(1) == Some(&0x5A) => {
            let largo = u16::from_le_bytes([*respuesta.get(4)?, *respuesta.get(5)?]) as usize;
            let contenido = respuesta.get(40..largo.checked_sub(16)?)?;
            if contenido.first() == Some(&0xAA) {
                Some(contenido.to_vec())
            } else {
                descifrar_ecb(&md5(CLAVE_FIRMA), contenido)
            }
        }
        _ => None,
    }
}

/// Midea envía los bytes como una lista de enteros con signo.
pub fn a_lista(datos: &[u8]) -> String {
    let partes: Vec<String> = datos.iter().map(|b| (*b as i8).to_string()).collect();
    partes.join(",")
}

pub fn de_lista(texto: &str) -> Option<Vec<u8>> {
    texto
        .split(',')
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().parse::<i32>().ok().map(|n| n as u8))
        .collect()
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::cifrado::{cifrar_ecb, de_hexa, hexa};

    const AIRE: u64 = 123456789012345;
    const MARCA: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

    fn base() -> Estado {
        Estado {
            encendido: true,
            objetivo: 23.0,
            modo: MODO_FRIO,
            ventilador: VENT_AUTO,
            giro: 0xC,
            eco: true,
            humedad_objetivo: Some(40),
            ..Default::default()
        }
    }

    #[test]
    fn crc_coincide_con_python() {
        assert_eq!(crc8(&[1, 2, 3]), 216);
    }

    #[test]
    fn consulta_coincide_con_python() {
        assert_eq!(
            hexa(&consulta_estado(1)),
            "aa21ac00000000000003418100ff03ff0002000000000000000000000000030169fe"
        );
    }

    #[test]
    fn cambio_coincide_con_python() {
        assert_eq!(
            hexa(&fijar_estado(&base(), false, 42)),
            "aa24ac00000000000002400347667f7f003c008000000000000000000028000000002a2f03"
        );
    }

    #[test]
    fn cambio_con_todos_los_bits_coincide_con_python() {
        let e = Estado {
            encendido: false,
            objetivo: 16.5,
            modo: MODO_CALOR,
            ventilador: VENT_ALTA,
            giro: 0,
            eco: false,
            turbo: true,
            sueno: true,
            purificador: true,
            seguir: true,
            fahrenheit: true,
            humedad_objetivo: None,
            ..Default::default()
        };
        assert_eq!(
            hexa(&fijar_estado(&e, true, 100)),
            "aa24ac00000000000002404290507f7f0030a020070000000000000004280000000064a2a5"
        );
    }

    #[test]
    fn paquete_coincide_con_python() {
        let trama = fijar_estado(&base(), false, 42);
        let paquete = armar_paquete(AIRE, &trama, MARCA);
        assert_eq!(
            hexa(&paquete),
            "5a5a01105d00200050000000010203040506070879df0d8648700000b60502040504000000000000aa24ac00000000000002400347667f7f003c008000000000000000000028000000002a2f037996a839810500f652d39dcc40be20fb"
        );
        assert_eq!(extraer_trama(&paquete).unwrap(), trama);
        assert!(a_lista(&paquete).starts_with("90,90,1,16,93,0,32,0,80,0,0,0,1,2,3,4,5,"));
    }

    #[test]
    fn lista_coincide_con_python() {
        assert_eq!(a_lista(&[0x00, 0x7F, 0x80, 0xFF]), "0,127,-128,-1");
        assert_eq!(de_lista("0,127,-128,-1").unwrap(), [0x00, 0x7F, 0x80, 0xFF]);
        let todos: Vec<u8> = (0..=255).collect();
        assert_eq!(de_lista(&a_lista(&todos)).unwrap(), todos);
        assert!(de_lista("1,no,3").is_none());
    }

    #[test]
    fn lee_respuesta_real_del_aire() {
        let trama = de_hexa("aa1eac00000000000003c00045667f7f00000000005d5f000000000000db33").unwrap();
        let e = leer_estado(&trama).unwrap();
        assert!(!e.encendido);
        assert_eq!(e.objetivo, 21.0);
        assert_eq!(e.modo, MODO_FRIO);
        assert_eq!(e.ventilador, VENT_AUTO);
        assert_eq!(e.interior, Some(21.5));
        assert_eq!(e.exterior, Some(22.5));
        assert!(e.pantalla);
        assert_eq!(e.error, 0);
        assert!(!e.filtro);
    }

    fn con_control(crudo: &str) -> Vec<u8> {
        let datos = de_hexa(crudo).unwrap();
        let mut cuerpo = datos.clone();
        cuerpo.push(crc8(&datos));
        trama(TRAMA_CONSULTA, &cuerpo)
    }

    #[test]
    fn lee_variantes_igual_que_python() {
        assert_eq!(
            hexa(&con_control("c00045667f7f00000000005f61000000000000")),
            "aa1eac00000000000003c00045667f7f00000000005f61000000000000000a"
        );
        let e = leer_estado(&con_control("c00145667f7f000c0000105f61000000000000")).unwrap();
        assert!(e.encendido);
        assert_eq!(e.giro, 12);
        assert_eq!((e.interior, e.exterior), (Some(22.5), Some(23.5)));

        let e = leer_estado(&con_control("c08164287f7f00000000285f61000000000000")).unwrap();
        assert!(e.encendido);
        assert_eq!((e.objetivo, e.modo, e.ventilador), (20.0, MODO_SECO, VENT_BAJA));
    }

    #[test]
    fn rechaza_tramas_danadas() {
        let mut trama = de_hexa("aa1eac00000000000003c00045667f7f00000000005d5f000000000000db33").unwrap();
        trama[12] ^= 0xFF;
        assert!(leer_estado(&trama).is_none());
        assert!(leer_estado(&[]).is_none());
        assert!(leer_estado(&consulta_estado(1)).is_none());
    }

    #[test]
    fn extrae_paquete_cifrado() {
        let consulta = consulta_estado(1);
        let cifrado = cifrar_ecb(&md5(CLAVE_FIRMA), &consulta).unwrap();
        let mut paquete = vec![0x5A, 0x5A, 0x01, 0x11];
        paquete.extend_from_slice(&((40 + cifrado.len() + 16) as u16).to_le_bytes());
        paquete.resize(40, 0);
        paquete.extend_from_slice(&cifrado);
        paquete.extend_from_slice(&[0; 16]);
        assert_eq!(extraer_trama(&paquete).unwrap(), consulta);
        assert!(extraer_trama(&[0, 1, 2, 3]).is_none());
    }

    #[test]
    fn cambios_se_acumulan_y_respetan_los_limites() {
        let mut c = Cambios { objetivo: Some(22.0), ..Default::default() };
        c.sumar(Cambios { objetivo: Some(99.0), encendido: Some(true), ..Default::default() });
        let mut e = base();
        e.encendido = false;
        c.aplicar_a(&mut e);
        assert!(e.encendido);
        assert_eq!(e.objetivo, TEMP_MAXIMA);
        assert_eq!(e.giro, 0xC);
    }
}
