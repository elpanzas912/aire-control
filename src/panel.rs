//! Panel flotante que se abre desde la bandeja. Dibujado a mano para gastar lo mínimo:
//! mientras está cerrado no existe nada gráfico en memoria.
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::time::Instant;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Memory::{GetProcessHeaps, HeapCompact, HEAP_FLAGS};
use windows::Win32::System::ProcessStatus::K32EmptyWorkingSet;
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::bandeja::{self, MENSAJE_BANDEJA};
use crate::hilo::{Compartido, Pedido, MENSAJE_VISTA};
use crate::historial::Historia;
use crate::inicio;
use crate::lienzo::{Alinear, Caja, Color, Fabricas, Lienzo, Peso};
use crate::motor::{nombre_modo, Fase, Vista, TRAMO_TEMPORIZADOR};
use crate::protocolo::*;
use crate::tiempo::{ahora, dia, dia_y_hora, duracion, hora};

const ANCHO: f32 = 380.0;
const M: f32 = 18.0;
const INTERIOR: f32 = ANCHO - M * 2.0;

const ALTO_CONTROL: f32 = 524.0;
const ALTO_DATOS: f32 = 640.0;
const ALTO_AJUSTES: f32 = 360.0;
const ALTO_LOGIN: f32 = 380.0;
const ALTO_ESPERA: f32 = 180.0;

const VELO: Color = Color(0x141B24, 0.66);
const SUPERFICIE: Color = Color::rgb(0x1B2430);
const TEXTO: Color = Color::rgb(0xF0F4F7);
const TEXTO_2: Color = Color::rgb(0xB4BEC8);
const TEXTO_3: Color = Color::rgb(0x8592A0);
const CONTROL: Color = Color(0xFFFFFF, 0.07);
const CONTROL_HOVER: Color = Color(0xFFFFFF, 0.13);
const CONTROL_APRETADO: Color = Color(0xFFFFFF, 0.04);
const RAYA: Color = Color(0xFFFFFF, 0.10);
const SOBRE_ACENTO: Color = Color::rgb(0x0D1117);
const ERROR: Color = Color::rgb(0xFF9A90);
const CAMPO: u32 = 0x2A3440;

// Serie del gráfico: pasan el validador de color sobre esta superficie
const LINEA_ADENTRO: Color = Color::rgb(0x3593E0);
const LINEA_AFUERA: Color = Color::rgb(0xD4722A);
const LINEA_OBJETIVO: Color = Color::rgb(0x8592A0);

const ICONO_POWER: u32 = 0xE7E8;
const ICONO_MAS: u32 = 0xE710;
const ICONO_MENOS: u32 = 0xE738;
const ICONO_AJUSTES: u32 = 0xE713;
const ICONO_ACTUALIZAR: u32 = 0xE72C;
const ICONO_CERRAR: u32 = 0xE711;
const ICONO_VOLVER: u32 = 0xE72B;
const ICONO_MARCA: u32 = 0xE73E;
// No existen en la fuente de iconos: se dibujan a mano
const ICONO_COPO: u32 = 0;
const ICONO_VIENTO: u32 = 1;

const WM_MOUSELEAVE: u32 = 0x02A3;
const TEMPORIZADOR_ENVIO: usize = 1;
const TEMPORIZADOR_RELOJ: usize = 2;
const ESPERA_ENVIO_MS: u32 = 350;

const MENU_ABRIR: u32 = 1;
const MENU_ENCENDER: u32 = 2;
const MENU_SUMAR: u32 = 3;
const MENU_CANCELAR: u32 = 4;
const MENU_INICIO: u32 = 5;
const MENU_SALIR: u32 = 6;

const MODOS: [(u8, &str, u32, u32); 5] = [
    (MODO_FRIO, "Frío", ICONO_COPO, 0x79C6F5),
    (MODO_CALOR, "Calor", 0xE706, 0xF5A663),
    (MODO_AUTO, "Auto", 0xE895, 0xA5D6A0),
    (MODO_SECO, "Seco", 0xEB42, 0x6FD3C7),
    (MODO_VENTILAR, "Ventilar", ICONO_VIENTO, 0xBDB6EC),
];
const VENTILADORES: [(u8, &str); 4] = [(VENT_AUTO, "Auto"), (VENT_BAJA, "Baja"), (VENT_MEDIA, "Media"), (VENT_ALTA, "Alta")];
const RANGOS: [&str; 3] = ["24 h", "7 días", "30 días"];

static CAMPO_CUENTA: AtomicIsize = AtomicIsize::new(0);
static CAMPO_CLAVE: AtomicIsize = AtomicIsize::new(0);
static MENSAJE_BARRA: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Pestana {
    Control,
    Datos,
    Ajustes,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Accion {
    Encender,
    Menos,
    Mas,
    Regla,
    Modo(u8),
    Ventilador(u8),
    SumarApagado,
    SumarEncendido,
    CancelarApagado,
    CancelarEncendido,
    Pestana(Pestana),
    Rango(u8),
    Actualizar,
    Exportar,
    InicioWindows,
    PotenciaMenos,
    PotenciaMas,
    AbrirCarpeta,
    SiguienteAire,
    CerrarSesion,
    Salir,
    Entrar,
    Recordar,
}

struct Zona {
    caja: Caja,
    accion: Accion,
}

pub struct Opciones {
    pub sin_ocultar: bool,
    pub pestana: Pestana,
    pub abrir: bool,
    pub titulo: &'static str,
}

pub struct App {
    ventana: HWND,
    pedidos: Sender<Pedido>,
    compartido: Compartido,
    opciones: Opciones,
    fabricas: Option<Fabricas>,
    lienzo: Option<Lienzo>,
    vista: Vista,
    tooltip: String,
    icono: Option<(String, u32, HICON)>,
    zonas: Vec<Zona>,
    caliente: Option<Accion>,
    apretado: Option<Accion>,
    foco: Option<usize>,
    teclado: bool,
    raton: (f32, f32),
    siguiendo_raton: bool,
    pestana: Pestana,
    rango: u8,
    pendientes: Cambios,
    en_vuelo: Cambios,
    numero_cambio: u64,
    visible: bool,
    oculto_en: Option<Instant>,
    recordar: bool,
    inicio_windows: bool,
    con_vidrio: bool,
    memoria_inicial_devuelta: bool,
    pincel_campo: HBRUSH,
    fuente_campo: HFONT,
}

fn grados(valor: Option<f32>) -> String {
    match valor {
        Some(v) => format!("{}°", decimal(v)),
        None => "—".to_string(),
    }
}

fn decimal(v: f32) -> String {
    let texto = if v.fract().abs() < 0.05 { format!("{v:.0}") } else { format!("{v:.1}") };
    texto.replace('.', ",")
}

fn acento_de(modo: u8) -> Color {
    MODOS.iter().find(|m| m.0 == modo).map(|m| Color::rgb(m.3)).unwrap_or(Color::rgb(0x79C6F5))
}

/// Con el panel cerrado el programa no necesita sus páginas de gráficos: se le devuelven a Windows.
fn devolver_memoria() {
    unsafe {
        let mut montones = [HANDLE::default(); 32];
        let cantidad = GetProcessHeaps(&mut montones) as usize;
        for monton in montones.iter().take(cantidad) {
            let _ = HeapCompact(*monton, HEAP_FLAGS(0));
        }
        let _ = K32EmptyWorkingSet(GetCurrentProcess());
    }
}

fn hexa_color(c: u32) -> COLORREF {
    COLORREF(((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF))
}

impl App {
    fn alto(&self) -> f32 {
        match self.vista_actual() {
            VistaPanel::Login => ALTO_LOGIN,
            VistaPanel::Espera => ALTO_ESPERA,
            VistaPanel::Normal => match self.pestana {
                Pestana::Control => ALTO_CONTROL,
                Pestana::Datos => ALTO_DATOS,
                Pestana::Ajustes if self.vista.aires.len() > 1 => ALTO_AJUSTES + 50.0,
                Pestana::Ajustes => ALTO_AJUSTES,
            },
        }
    }

    fn vista_actual(&self) -> VistaPanel {
        match self.vista.fase {
            Fase::SinCuenta | Fase::CuentaRechazada => VistaPanel::Login,
            Fase::Conectando => VistaPanel::Espera,
            Fase::Conectado if self.vista.estado.is_none() => VistaPanel::Espera,
            Fase::Conectado => VistaPanel::Normal,
        }
    }

    fn estado_visible(&self) -> Option<Estado> {
        let mut e = self.vista.estado.clone()?;
        self.en_vuelo.aplicar_a(&mut e);
        self.pendientes.aplicar_a(&mut e);
        Some(e)
    }

    fn enviar(&self, pedido: Pedido) {
        let _ = self.pedidos.send(pedido);
    }

    fn redibujar(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.ventana), None, false);
        }
    }

    // Ventana

    fn mostrar(&mut self) {
        self.visible = true;
        self.inicio_windows = inicio::activo();
        self.ubicar();
        unsafe {
            let _ = ShowWindow(self.ventana, SW_SHOW);
            let _ = SetForegroundWindow(self.ventana);
            let _ = SetFocus(Some(self.ventana));
            let _ = SetTimer(Some(self.ventana), TEMPORIZADOR_RELOJ, 1000, None);
        }
        self.enviar(Pedido::PanelAbierto(true));
        if self.pestana == Pestana::Datos {
            self.enviar(Pedido::Historia(self.rango));
        }
        self.sincronizar_campos();
        self.redibujar();
    }

    fn ocultar(&mut self) {
        if !self.visible || self.opciones.sin_ocultar {
            return;
        }
        self.visible = false;
        self.oculto_en = Some(Instant::now());
        self.lienzo = None;
        self.foco = None;
        self.teclado = false;
        self.caliente = None;
        unsafe {
            let _ = ShowWindow(self.ventana, SW_HIDE);
            let _ = KillTimer(Some(self.ventana), TEMPORIZADOR_RELOJ);
        }
        self.enviar(Pedido::PanelAbierto(false));
        devolver_memoria();
    }

    fn alternar(&mut self) {
        if self.visible {
            self.ocultar();
        } else if self.oculto_en.is_none_or(|t| t.elapsed().as_millis() > 300) {
            // El clic en el ícono primero cierra el panel por perder el foco: no hay que reabrirlo
            self.mostrar();
        }
    }

    fn ubicar(&self) {
        unsafe {
            let zona = bandeja::rectangulo(self.ventana).unwrap_or_else(|| {
                let mut punto = POINT::default();
                let _ = GetCursorPos(&mut punto);
                RECT { left: punto.x, top: punto.y, right: punto.x + 1, bottom: punto.y + 1 }
            });
            let monitor = MonitorFromRect(&zona, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(monitor, &mut info);
            let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            let escala = dpi_x as f32 / 96.0;
            let ancho = (ANCHO * escala).round() as i32;
            let alto = (self.alto() * escala).round() as i32;
            let margen = (12.0 * escala) as i32;
            let (trabajo, pantalla) = (info.rcWork, info.rcMonitor);
            let x = if trabajo.left > pantalla.left { trabajo.left + margen } else { trabajo.right - ancho - margen };
            let y = if trabajo.top > pantalla.top { trabajo.top + margen } else { trabajo.bottom - alto - margen };
            let _ = SetWindowPos(self.ventana, Some(HWND_TOPMOST), x, y, ancho, alto, SWP_NOACTIVATE);
        }
    }

    fn escala(&self) -> f32 {
        unsafe { GetDpiForWindow(self.ventana) as f32 / 96.0 }
    }

    fn fijar_vidrio(&mut self, activo: bool) {
        if self.con_vidrio == activo {
            return;
        }
        let fondo = if activo { DWMSBT_TRANSIENTWINDOW } else { DWMSBT_NONE };
        unsafe {
            // Windows 10 y las primeras versiones de Windows 11 no tienen acrílico: ahí queda el fondo sólido
            let aceptado =
                DwmSetWindowAttribute(self.ventana, DWMWA_SYSTEMBACKDROP_TYPE, &fondo as *const _ as *const _, 4).is_ok();
            self.con_vidrio = activo && aceptado;
            let borde = if self.con_vidrio { -1 } else { 0 };
            let margenes = MARGINS { cxLeftWidth: borde, cxRightWidth: borde, cyTopHeight: borde, cyBottomHeight: borde };
            let _ = DwmExtendFrameIntoClientArea(self.ventana, &margenes);
        }
    }

    fn sincronizar_campos(&mut self) {
        let en_login = self.vista_actual() == VistaPanel::Login && self.visible;
        self.fijar_vidrio(self.vista_actual() != VistaPanel::Login);
        let escala = self.escala();
        let colocar = |campo: &AtomicIsize, y: f32| unsafe {
            let hwnd = HWND(campo.load(Ordering::Relaxed) as *mut _);
            if hwnd.0.is_null() {
                return;
            }
            let _ = SetWindowPos(
                hwnd,
                None,
                ((M + 8.0) * escala) as i32,
                ((y + 7.0) * escala) as i32,
                ((INTERIOR - 16.0) * escala) as i32,
                (22.0 * escala) as i32,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = ShowWindow(hwnd, if en_login { SW_SHOW } else { SW_HIDE });
        };
        colocar(&CAMPO_CUENTA, Y_CAMPO_CUENTA);
        colocar(&CAMPO_CLAVE, Y_CAMPO_CLAVE);
        if en_login {
            let campo = if Self::texto_de_campo(&CAMPO_CUENTA).is_empty() { &CAMPO_CUENTA } else { &CAMPO_CLAVE };
            unsafe {
                let _ = SetFocus(Some(HWND(campo.load(Ordering::Relaxed) as *mut _)));
            }
        }
    }

    fn crear_fuente_campos(&mut self) {
        unsafe {
            if !self.fuente_campo.is_invalid() {
                let _ = DeleteObject(self.fuente_campo.into());
            }
            self.fuente_campo = CreateFontW(
                -(14.0 * self.escala()) as i32, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0,
                DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Bahnschrift"),
            );
            for campo in [&CAMPO_CUENTA, &CAMPO_CLAVE] {
                let hwnd = HWND(campo.load(Ordering::Relaxed) as *mut _);
                SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(self.fuente_campo.0 as usize)), Some(LPARAM(1)));
            }
        }
    }

    fn texto_de_campo(campo: &AtomicIsize) -> String {
        unsafe {
            let hwnd = HWND(campo.load(Ordering::Relaxed) as *mut _);
            let largo = GetWindowTextLengthW(hwnd);
            let mut buffer = vec![0u16; largo as usize + 1];
            let copiados = GetWindowTextW(hwnd, &mut buffer);
            String::from_utf16_lossy(&buffer[..copiados as usize])
        }
    }

    fn limpiar_clave() {
        unsafe {
            let hwnd = HWND(CAMPO_CLAVE.load(Ordering::Relaxed) as *mut _);
            let _ = SetWindowTextW(hwnd, PCWSTR::null());
        }
    }

    // Datos que llegan del hilo

    fn al_recibir_vista(&mut self) {
        let fase_anterior = self.vista_actual();
        let (vista, avisos, tooltip) = {
            let mut p = self.compartido.lock().unwrap();
            (p.vista.clone(), std::mem::take(&mut p.avisos), p.tooltip.clone())
        };
        let alto_anterior = self.alto();
        self.vista = vista;
        self.tooltip = tooltip;
        if self.vista.cambios_atendidos >= self.numero_cambio {
            self.en_vuelo = Cambios::default();
        }
        self.actualizar_icono();
        for aviso in avisos {
            bandeja::avisar(self.ventana, &aviso.titulo, &aviso.texto);
        }
        if !self.visible && !self.memoria_inicial_devuelta {
            self.memoria_inicial_devuelta = true;
            devolver_memoria();
        }
        if self.visible {
            if fase_anterior != self.vista_actual() || alto_anterior != self.alto() {
                self.ubicar();
                self.sincronizar_campos();
            }
            if self.vista.fase == Fase::CuentaRechazada && fase_anterior != VistaPanel::Login {
                Self::limpiar_clave();
            }
            if fase_anterior != VistaPanel::Normal && self.vista_actual() == VistaPanel::Normal {
                Self::limpiar_clave();
                unsafe {
                    let _ = SetFocus(Some(self.ventana));
                }
            }
            self.redibujar();
        }
    }

    fn actualizar_icono(&mut self) {
        let e = self.vista.estado.as_ref();
        let texto = e
            .and_then(|e| e.interior)
            .map(|t| format!("{}", t.round() as i32))
            .unwrap_or_else(|| "–".to_string());
        let color = match e {
            Some(e) if e.encendido => acento_de(e.modo).0,
            _ => 0xA3AEB9,
        };
        let cambio = self.icono.as_ref().is_none_or(|(t, c, _)| *t != texto || *c != color);
        if cambio {
            if let Some(nuevo) = bandeja::crear_icono(&texto, color) {
                if let Some((_, _, viejo)) = self.icono.replace((texto, color, nuevo)) {
                    bandeja::actualizar(self.ventana, nuevo, &self.tooltip);
                    unsafe {
                        let _ = DestroyIcon(viejo);
                    }
                } else {
                    bandeja::agregar(self.ventana, nuevo, &self.tooltip);
                }
                return;
            }
        }
        if let Some((_, _, icono)) = &self.icono {
            bandeja::actualizar(self.ventana, *icono, &self.tooltip);
        }
    }

    // Acciones

    fn cambiar(&mut self, cambios: Cambios) {
        self.pendientes.sumar(cambios);
        unsafe {
            let _ = SetTimer(Some(self.ventana), TEMPORIZADOR_ENVIO, ESPERA_ENVIO_MS, None);
        }
        self.redibujar();
    }

    fn enviar_pendientes(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.ventana), TEMPORIZADOR_ENVIO);
        }
        if self.pendientes.vacio() {
            return;
        }
        self.numero_cambio += 1;
        self.en_vuelo.sumar(self.pendientes);
        self.enviar(Pedido::Cambiar(self.pendientes, self.numero_cambio));
        self.pendientes = Cambios::default();
    }

    fn mover_objetivo(&mut self, pasos: f32) {
        let Some(e) = self.estado_visible() else { return };
        if self.vista_actual() != VistaPanel::Normal || e.modo == MODO_VENTILAR {
            return;
        }
        let objetivo = (e.objetivo + pasos).clamp(TEMP_MINIMA, TEMP_MAXIMA);
        if objetivo != e.objetivo {
            self.cambiar(Cambios { objetivo: Some(objetivo), ..Default::default() });
        }
    }

    fn fijar_desde_regla(&mut self, x: f32) {
        let t = TEMP_MINIMA + ((x - REGLA_X) / REGLA_ANCHO * (TEMP_MAXIMA - TEMP_MINIMA)).round();
        let t = t.clamp(TEMP_MINIMA, TEMP_MAXIMA);
        if self.estado_visible().is_some_and(|e| e.objetivo != t && e.modo != MODO_VENTILAR) {
            self.cambiar(Cambios { objetivo: Some(t), ..Default::default() });
        }
    }

    fn entrar(&mut self) {
        let cuenta = Self::texto_de_campo(&CAMPO_CUENTA).trim().to_string();
        let contrasena = Self::texto_de_campo(&CAMPO_CLAVE);
        if cuenta.is_empty() || contrasena.is_empty() {
            self.vista.error = Some("Completá el email y la contraseña.".to_string());
            self.redibujar();
            return;
        }
        self.vista.fase = Fase::Conectando;
        self.vista.error = None;
        self.enviar(Pedido::Conectar { cuenta, contrasena, recordar: self.recordar });
        self.ubicar();
        self.sincronizar_campos();
        self.redibujar();
    }

    fn ejecutar(&mut self, accion: Accion) {
        match accion {
            Accion::Encender => {
                if let Some(e) = self.estado_visible() {
                    self.cambiar(Cambios { encendido: Some(!e.encendido), ..Default::default() });
                }
            }
            Accion::Menos => self.mover_objetivo(-1.0),
            Accion::Mas => self.mover_objetivo(1.0),
            Accion::Regla => {}
            Accion::Modo(m) => self.cambiar(Cambios { modo: Some(m), ..Default::default() }),
            Accion::Ventilador(v) => self.cambiar(Cambios { ventilador: Some(v), ..Default::default() }),
            Accion::SumarApagado => {
                self.enviar_pendientes();
                self.enviar(Pedido::SumarApagado);
            }
            Accion::SumarEncendido => self.enviar(Pedido::SumarEncendido),
            Accion::CancelarApagado => self.enviar(Pedido::CancelarApagado),
            Accion::CancelarEncendido => self.enviar(Pedido::CancelarEncendido),
            Accion::Pestana(p) => {
                self.pestana = p;
                self.foco = None;
                if p == Pestana::Datos {
                    self.enviar(Pedido::Historia(self.rango));
                }
                self.ubicar();
                self.redibujar();
            }
            Accion::Rango(r) => {
                self.rango = r;
                self.enviar(Pedido::Historia(r));
                self.redibujar();
            }
            Accion::Actualizar => {
                self.enviar(Pedido::Actualizar);
                if self.pestana == Pestana::Datos {
                    self.enviar(Pedido::Historia(self.rango));
                }
            }
            Accion::Exportar => self.enviar(Pedido::ExportarCsv),
            Accion::InicioWindows => {
                if inicio::fijar(!self.inicio_windows) {
                    self.inicio_windows = !self.inicio_windows;
                }
                self.redibujar();
            }
            Accion::PotenciaMenos => self.enviar(Pedido::Potencia(self.vista.potencia - 100)),
            Accion::PotenciaMas => self.enviar(Pedido::Potencia(self.vista.potencia + 100)),
            Accion::AbrirCarpeta => {
                let _ = std::process::Command::new("explorer").arg(inicio::carpeta_datos()).spawn();
            }
            Accion::SiguienteAire => self.enviar(Pedido::SiguienteAire),
            Accion::CerrarSesion => {
                self.enviar(Pedido::Olvidar);
                self.vista.fase = Fase::SinCuenta;
                self.pestana = Pestana::Control;
                self.ubicar();
                self.sincronizar_campos();
                self.redibujar();
            }
            Accion::Salir => self.salir(),
            Accion::Entrar => self.entrar(),
            Accion::Recordar => {
                self.recordar = !self.recordar;
                self.redibujar();
            }
        }
    }

    fn salir(&mut self) {
        self.enviar(Pedido::Salir);
        bandeja::quitar(self.ventana);
        unsafe {
            let _ = DestroyWindow(self.ventana);
        }
    }

    fn menu_bandeja(&mut self) {
        let encendido = self.vista.estado.as_ref().is_some_and(|e| e.encendido);
        let hay_temporizador = self.vista.apagar_en.is_some() || self.vista.encender_en.is_some();
        let mut opciones = vec![
            (MENU_ABRIR, "Abrir panel", false),
            (0, "", false),
            (MENU_ENCENDER, if encendido { "Apagar" } else { "Prender" }, false),
            (MENU_SUMAR, "Mantener prendido +30 min", false),
        ];
        if hay_temporizador {
            opciones.push((MENU_CANCELAR, "Cancelar temporizadores", false));
        }
        self.inicio_windows = inicio::activo();
        opciones.push((0, "", false));
        opciones.push((MENU_INICIO, "Iniciar con Windows", self.inicio_windows));
        opciones.push((MENU_SALIR, "Salir", false));
        match bandeja::mostrar_menu(self.ventana, &opciones) {
            MENU_ABRIR => self.mostrar(),
            MENU_ENCENDER => {
                self.enviar(Pedido::Cambiar(
                    Cambios { encendido: Some(!encendido), ..Default::default() },
                    self.numero_cambio,
                ));
            }
            MENU_SUMAR => self.enviar(Pedido::SumarApagado),
            MENU_CANCELAR => {
                self.enviar(Pedido::CancelarApagado);
                self.enviar(Pedido::CancelarEncendido);
            }
            MENU_INICIO => self.ejecutar(Accion::InicioWindows),
            MENU_SALIR => self.salir(),
            _ => {}
        }
    }

    // Ratón y teclado

    fn zona_en(&self, x: f32, y: f32) -> Option<Accion> {
        self.zonas.iter().rev().find(|z| z.caja.contiene(x, y)).map(|z| z.accion)
    }

    fn al_mover(&mut self, x: f32, y: f32) {
        self.raton = (x, y);
        if !self.siguiendo_raton {
            let mut pedido = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.ventana,
                dwHoverTime: 0,
            };
            unsafe {
                let _ = TrackMouseEvent(&mut pedido);
            }
            self.siguiendo_raton = true;
        }
        if self.apretado == Some(Accion::Regla) {
            self.fijar_desde_regla(x);
        }
        let nuevo = self.zona_en(x, y);
        let en_grafico = self.pestana == Pestana::Datos && GRAFICO.contiene(x, y);
        if nuevo != self.caliente || en_grafico || self.pestana == Pestana::Datos {
            self.caliente = nuevo;
            self.redibujar();
        }
    }

    fn al_apretar(&mut self, x: f32, y: f32) {
        self.teclado = false;
        self.apretado = self.zona_en(x, y);
        if self.apretado == Some(Accion::Regla) {
            self.fijar_desde_regla(x);
        }
        unsafe {
            SetCapture(self.ventana);
        }
        self.redibujar();
    }

    fn al_soltar(&mut self, x: f32, y: f32) {
        unsafe {
            let _ = ReleaseCapture();
        }
        let apretado = self.apretado.take();
        if let Some(accion) = apretado {
            if self.zona_en(x, y) == Some(accion) {
                self.ejecutar(accion);
            }
        }
        self.redibujar();
    }

    fn al_tecla(&mut self, tecla: u16) {
        match VIRTUAL_KEY(tecla) {
            VK_ESCAPE => self.ocultar(),
            VK_UP => self.mover_objetivo(1.0),
            VK_DOWN => self.mover_objetivo(-1.0),
            VK_TAB if !self.zonas.is_empty() => {
                self.teclado = true;
                let atras = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
                let total = self.zonas.len();
                self.foco = Some(match (self.foco, atras) {
                    (None, false) => 0,
                    (None, true) => total - 1,
                    (Some(i), false) => (i + 1) % total,
                    (Some(i), true) => (i + total - 1) % total,
                });
                self.redibujar();
            }
            VK_RETURN | VK_SPACE => {
                if let Some(z) = self.foco.and_then(|i| self.zonas.get(i)) {
                    let accion = z.accion;
                    self.ejecutar(accion);
                }
            }
            _ => {}
        }
    }

    // Pintado

    fn pintar(&mut self) {
        if self.fabricas.is_none() {
            self.fabricas = Fabricas::nuevas().ok();
        }
        if self.lienzo.is_none() {
            self.lienzo = self.fabricas.as_ref().and_then(|f| Lienzo::nuevo(self.ventana, f).ok());
        }
        let Some(mut l) = self.lienzo.take() else { return };
        self.zonas.clear();
        l.empezar();
        let fondo = if self.con_vidrio { VELO } else { SUPERFICIE };
        l.caja(Caja::new(0.0, 0.0, ANCHO, 2000.0), 0.0, fondo);
        match self.vista_actual() {
            VistaPanel::Login => self.pintar_login(&mut l),
            VistaPanel::Espera => self.pintar_espera(&mut l),
            VistaPanel::Normal => {
                self.pintar_cabecera(&mut l);
                match self.pestana {
                    Pestana::Control => self.pintar_control(&mut l),
                    Pestana::Datos => self.pintar_datos(&mut l),
                    Pestana::Ajustes => self.pintar_ajustes(&mut l),
                }
            }
        }
        self.pintar_foco(&mut l);
        if l.terminar().is_ok() {
            self.lienzo = Some(l);
        }
    }

    fn pintar_foco(&self, l: &mut Lienzo) {
        if !self.teclado {
            return;
        }
        if let Some(z) = self.foco.and_then(|i| self.zonas.get(i)) {
            l.borde(z.caja, 6.0, 2.0, TEXTO);
        }
    }

    fn zona(&mut self, caja: Caja, accion: Accion) {
        self.zonas.push(Zona { caja, accion });
    }

    fn fondo_de(&self, accion: Accion) -> Color {
        if self.apretado == Some(accion) {
            CONTROL_APRETADO
        } else if self.caliente == Some(accion) {
            CONTROL_HOVER
        } else {
            CONTROL
        }
    }

    fn boton_icono(&mut self, l: &mut Lienzo, c: Caja, icono: u32, tamano: f32, accion: Accion, activo: bool) {
        if activo {
            l.caja(c, 6.0, self.fondo_de(accion));
            l.icono(icono, c, tamano, TEXTO);
            self.zona(c, accion);
        } else {
            l.caja(c, 6.0, Color(0xFFFFFF, 0.03));
            l.icono(icono, c, tamano, TEXTO_3);
        }
    }

    fn pintar_cabecera(&mut self, l: &mut Lienzo) {
        l.texto(self.opciones.titulo, Caja::new(M, 12.0, 170.0, 30.0), 15.0, Peso::Medio, Alinear::Izquierda, TEXTO);
        if self.pestana == Pestana::Ajustes {
            self.boton_icono(l, Caja::new(ANCHO - M - 30.0, 13.0, 30.0, 28.0), ICONO_VOLVER, 13.0, Accion::Pestana(Pestana::Control), true);
            return;
        }
        let ancho_boton = 62.0;
        let x = ANCHO - M - 30.0 - 6.0 - ancho_boton * 2.0;
        for (i, (nombre, p)) in [("Control", Pestana::Control), ("Datos", Pestana::Datos)].into_iter().enumerate() {
            let c = Caja::new(x + i as f32 * ancho_boton, 13.0, ancho_boton, 28.0);
            let accion = Accion::Pestana(p);
            if self.pestana == p {
                l.caja(c, 6.0, Color(0xFFFFFF, 0.16));
                l.texto(nombre, c, 12.0, Peso::Medio, Alinear::Centro, TEXTO);
            } else {
                if self.caliente == Some(accion) {
                    l.caja(c, 6.0, CONTROL);
                }
                l.texto(nombre, c, 12.0, Peso::Normal, Alinear::Centro, TEXTO_2);
            }
            self.zona(c, accion);
        }
        self.boton_icono(l, Caja::new(ANCHO - M - 30.0, 13.0, 30.0, 28.0), ICONO_AJUSTES, 13.0, Accion::Pestana(Pestana::Ajustes), true);
    }

    fn pintar_control(&mut self, l: &mut Lienzo) {
        let Some(e) = self.estado_visible() else { return };
        let acento = acento_de(e.modo);
        let ventilando = e.modo == MODO_VENTILAR;

        // La cifra grande: lo que se está pidiendo
        let texto_objetivo = if ventilando { "—".to_string() } else { format!("{}°", decimal(e.objetivo)) };
        let color_objetivo = if e.encendido && !ventilando { acento } else { TEXTO_2 };
        l.texto(&texto_objetivo, Caja::new(M - 3.0, 46.0, 190.0, 78.0), 68.0, Peso::Liviano, Alinear::Izquierda, color_objetivo);
        let estado = match (e.encendido, ventilando) {
            (false, _) => "Apagado".to_string(),
            (true, true) => "Prendido, solo ventilador".to_string(),
            (true, false) => format!("Prendido en {}", nombre_modo(e.modo)),
        };
        l.texto(&estado, Caja::new(M, 122.0, 190.0, 18.0), 12.0, Peso::Normal, Alinear::Izquierda, TEXTO_2);

        let derecha = ANCHO - M - 110.0;
        l.texto("Adentro", Caja::new(derecha, 58.0, 110.0, 16.0), 11.0, Peso::Normal, Alinear::Derecha, TEXTO_3);
        l.texto(&grados(e.interior), Caja::new(derecha, 73.0, 110.0, 26.0), 20.0, Peso::Normal, Alinear::Derecha, TEXTO);
        l.texto("Afuera", Caja::new(derecha, 100.0, 110.0, 16.0), 11.0, Peso::Normal, Alinear::Derecha, TEXTO_3);
        l.texto(&grados(e.exterior), Caja::new(derecha, 115.0, 110.0, 26.0), 20.0, Peso::Normal, Alinear::Derecha, TEXTO);

        self.pintar_regla(l, &e, acento);

        // Prender, y ajuste fino
        let y = 204.0;
        self.boton_icono(l, Caja::new(M, y, 52.0, 44.0), ICONO_MENOS, 15.0, Accion::Menos, !ventilando && e.objetivo > TEMP_MINIMA);
        self.boton_icono(l, Caja::new(ANCHO - M - 52.0, y, 52.0, 44.0), ICONO_MAS, 15.0, Accion::Mas, !ventilando && e.objetivo < TEMP_MAXIMA);
        let poder = Caja::new(M + 58.0, y, INTERIOR - 116.0, 44.0);
        let (fondo, letra, nombre) = if e.encendido {
            (self.fondo_de(Accion::Encender), TEXTO, "Apagar")
        } else {
            let alfa = if self.apretado == Some(Accion::Encender) { 0.75 } else { 1.0 };
            (acento.con_alfa(alfa), SOBRE_ACENTO, "Prender")
        };
        l.caja(poder, 8.0, fondo);
        let ancho_texto = l.medir(nombre, 14.0, Peso::Medio);
        let inicio_x = poder.x + (poder.w - ancho_texto - 24.0) / 2.0;
        l.icono(ICONO_POWER, Caja::new(inicio_x, poder.y, 18.0, poder.h), 14.0, letra);
        l.texto(nombre, Caja::new(inicio_x + 24.0, poder.y, ancho_texto + 4.0, poder.h), 14.0, Peso::Medio, Alinear::Izquierda, letra);
        self.zona(poder, Accion::Encender);

        // Modo
        let y = 258.0;
        let separacion = 4.0;
        let ancho_modo = (INTERIOR - separacion * 4.0) / 5.0;
        for (i, (modo, nombre, icono, color)) in MODOS.iter().enumerate() {
            let c = Caja::new(M + i as f32 * (ancho_modo + separacion), y, ancho_modo, 56.0);
            let accion = Accion::Modo(*modo);
            let elegido = e.modo == *modo;
            let (fondo, letra) = if elegido { (Color::rgb(*color), SOBRE_ACENTO) } else { (self.fondo_de(accion), TEXTO) };
            l.caja(c, 8.0, fondo);
            let zona_icono = Caja::new(c.x, c.y + 7.0, c.w, 24.0);
            match *icono {
                ICONO_COPO => dibujar_copo(l, zona_icono.x + zona_icono.w / 2.0, zona_icono.y + 12.0, 9.0, letra),
                ICONO_VIENTO => dibujar_viento(l, zona_icono.x + zona_icono.w / 2.0, zona_icono.y + 12.0, letra),
                codigo => l.icono(codigo, zona_icono, 16.0, letra),
            }
            l.texto(nombre, Caja::new(c.x, c.y + 32.0, c.w, 18.0), 11.0, Peso::Normal, Alinear::Centro, letra);
            self.zona(c, accion);
        }

        // Ventilador
        let y = 322.0;
        l.texto("Ventilador", Caja::new(M, y, 74.0, 32.0), 12.0, Peso::Normal, Alinear::Izquierda, TEXTO_2);
        let ancho_vent = (INTERIOR - 76.0 - separacion * 3.0) / 4.0;
        for (i, (velocidad, nombre)) in VENTILADORES.iter().enumerate() {
            let c = Caja::new(M + 76.0 + i as f32 * (ancho_vent + separacion), y, ancho_vent, 32.0);
            let accion = Accion::Ventilador(*velocidad);
            let elegido = e.ventilador == *velocidad;
            let (fondo, letra) = if elegido { (acento, SOBRE_ACENTO) } else { (self.fondo_de(accion), TEXTO) };
            l.caja(c, 6.0, fondo);
            l.texto(nombre, c, 12.0, Peso::Normal, Alinear::Centro, letra);
            self.zona(c, accion);
        }

        l.linea(M, 368.0, ANCHO - M, 368.0, 1.0, RAYA);

        // Temporizadores: cada toque suma un tramo de 30 minutos
        let t = ahora();
        let ancho_tarjeta = (INTERIOR - 8.0) / 2.0;
        self.pintar_temporizador(
            l,
            Caja::new(M, 378.0, ancho_tarjeta, 104.0),
            "Mantener prendido",
            "se apaga a las",
            self.vista.apagar_en,
            t,
            acento,
            Accion::SumarApagado,
            Accion::CancelarApagado,
        );
        self.pintar_temporizador(
            l,
            Caja::new(M + ancho_tarjeta + 8.0, 378.0, ancho_tarjeta, 104.0),
            "Prender más tarde",
            "se prende a las",
            self.vista.encender_en,
            t,
            acento,
            Accion::SumarEncendido,
            Accion::CancelarEncendido,
        );

        self.pintar_pie(l, 490.0);
    }

    fn pintar_regla(&mut self, l: &mut Lienzo, e: &Estado, acento: Color) {
        let y = REGLA_Y;
        let x_de = |t: f32| REGLA_X + (t.clamp(TEMP_MINIMA, TEMP_MAXIMA) - TEMP_MINIMA) / (TEMP_MAXIMA - TEMP_MINIMA) * REGLA_ANCHO;

        let mut grado = TEMP_MINIMA as i32;
        while grado <= TEMP_MAXIMA as i32 {
            let x = x_de(grado as f32);
            let mayor = grado % 2 == 0;
            let alto = if mayor { 6.0 } else { 3.0 };
            l.linea(x, y - alto, x, y + alto, 1.0, Color(0xFFFFFF, if mayor { 0.28 } else { 0.16 }));
            grado += 1;
        }
        let ventilando = e.modo == MODO_VENTILAR;
        if let Some(interior) = e.interior {
            let x_cuarto = x_de(interior);
            if !ventilando {
                // Tramo entre el objetivo y el cuarto: lo que le falta al aire
                l.linea(x_de(e.objetivo), y, x_cuarto, y, 4.0, acento.con_alfa(if e.encendido { 0.55 } else { 0.2 }));
            }
            l.circulo(x_cuarto, y, 6.0, SUPERFICIE);
            l.anillo(x_cuarto, y, 4.0, 2.0, TEXTO);
        }
        if !ventilando {
            let x = x_de(e.objetivo);
            l.circulo(x, y, 10.0, SUPERFICIE);
            l.circulo(x, y, 8.0, if e.encendido { acento } else { TEXTO_2 });
        }
        let etiquetas = y + 11.0;
        l.texto("16°", Caja::new(M, etiquetas, 40.0, 16.0), 10.5, Peso::Normal, Alinear::Izquierda, TEXTO_3);
        l.texto("30°", Caja::new(ANCHO - M - 40.0, etiquetas, 40.0, 16.0), 10.5, Peso::Normal, Alinear::Derecha, TEXTO_3);
        // Qué significa cada marca
        let centro = ANCHO / 2.0;
        l.circulo(centro - 62.0, etiquetas + 8.0, 4.0, if e.encendido && !ventilando { acento } else { TEXTO_2 });
        l.texto("objetivo", Caja::new(centro - 54.0, etiquetas, 50.0, 16.0), 10.5, Peso::Normal, Alinear::Izquierda, TEXTO_3);
        l.anillo(centro + 8.0, etiquetas + 8.0, 3.0, 1.5, TEXTO);
        l.texto("el cuarto", Caja::new(centro + 15.0, etiquetas, 52.0, 16.0), 10.5, Peso::Normal, Alinear::Izquierda, TEXTO_3);
        if !ventilando {
            self.zona(Caja::new(REGLA_X - 10.0, y - 14.0, REGLA_ANCHO + 20.0, 28.0), Accion::Regla);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pintar_temporizador(
        &mut self,
        l: &mut Lienzo,
        c: Caja,
        titulo: &str,
        nota: &str,
        cuando: Option<i64>,
        t: i64,
        acento: Color,
        sumar: Accion,
        cancelar: Accion,
    ) {
        let activo = cuando.is_some();
        l.caja(c, 8.0, self.fondo_de(sumar));
        self.zona(c, sumar);
        l.texto(titulo, Caja::new(c.x + 10.0, c.y + 7.0, c.w - 38.0, 18.0), 12.0, Peso::Medio, Alinear::Izquierda, TEXTO);
        match cuando {
            None => {
                l.texto("+30 min", Caja::new(c.x + 10.0, c.y + 28.0, c.w - 20.0, 32.0), 22.0, Peso::Liviano, Alinear::Izquierda, TEXTO);
                l.texto("Cada toque suma 30", Caja::new(c.x + 10.0, c.y + 62.0, c.w - 20.0, 16.0), 11.0, Peso::Normal, Alinear::Izquierda, TEXTO_3);
            }
            Some(fin) => {
                let resta = (fin - t).max(0);
                l.texto(&duracion(resta), Caja::new(c.x + 10.0, c.y + 28.0, c.w - 20.0, 32.0), 22.0, Peso::Liviano, Alinear::Izquierda, TEXTO);
                l.texto(&format!("{nota} {}", hora(fin)), Caja::new(c.x + 10.0, c.y + 62.0, c.w - 20.0, 16.0), 11.0, Peso::Normal, Alinear::Izquierda, TEXTO_2);

                // Un tramo por cada 30 minutos; el primero se va vaciando
                let tramos = ((resta + TRAMO_TEMPORIZADOR - 1) / TRAMO_TEMPORIZADOR).max(1) as i32;
                let visibles = tramos.min(8);
                let (x0, y0) = (c.x + 10.0, c.y + 88.0);
                let total = c.w - 20.0;
                let hueco = 2.0;
                let ancho = (total - hueco * (visibles - 1) as f32) / visibles as f32;
                let parcial = resta % TRAMO_TEMPORIZADOR;
                let parte = if parcial == 0 { 1.0 } else { parcial as f32 / TRAMO_TEMPORIZADOR as f32 };
                for i in 0..visibles {
                    let x = x0 + i as f32 * (ancho + hueco);
                    let barra = Caja::new(x, y0, ancho, 5.0);
                    l.caja(barra, 2.0, Color(0xFFFFFF, 0.12));
                    let lleno = if i == 0 { ancho * parte } else { ancho };
                    if lleno >= 1.0 {
                        l.caja(Caja::new(x, y0, lleno, 5.0), 2.0, acento);
                    }
                }
            }
        }
        if activo {
            let x = Caja::new(c.x + c.w - 28.0, c.y + 5.0, 24.0, 24.0);
            if self.caliente == Some(cancelar) {
                l.caja(x, 6.0, CONTROL_HOVER);
            }
            l.icono(ICONO_CERRAR, x, 10.0, TEXTO_2);
            self.zona(x, cancelar);
        }
    }

    fn pintar_pie(&mut self, l: &mut Lienzo, y: f32) {
        let (texto, color) = match &self.vista.error {
            Some(error) => (error.clone(), ERROR),
            None if self.vista.actualizado > 0 => (format!("Actualizado a las {}", hora(self.vista.actualizado)), TEXTO_3),
            None => (String::new(), TEXTO_3),
        };
        l.texto_recortado(&texto, Caja::new(M, y, INTERIOR - 36.0, 26.0), 11.0, color);
        self.boton_icono(l, Caja::new(ANCHO - M - 28.0, y, 28.0, 26.0), ICONO_ACTUALIZAR, 11.0, Accion::Actualizar, true);
    }

    fn pintar_datos(&mut self, l: &mut Lienzo) {
        // Un solo filtro, arriba de todo lo que afecta
        let ancho = 64.0;
        for (i, nombre) in RANGOS.iter().enumerate() {
            let c = Caja::new(M + i as f32 * (ancho + 4.0), 50.0, ancho, 28.0);
            let accion = Accion::Rango(i as u8);
            let elegido = self.rango == i as u8;
            let fondo = if elegido { Color(0xFFFFFF, 0.16) } else { self.fondo_de(accion) };
            l.caja(c, 6.0, fondo);
            l.texto(nombre, c, 12.0, if elegido { Peso::Medio } else { Peso::Normal }, Alinear::Centro, TEXTO);
            self.zona(c, accion);
        }
        let exportar = Caja::new(ANCHO - M - 96.0, 50.0, 96.0, 28.0);
        l.caja(exportar, 6.0, self.fondo_de(Accion::Exportar));
        l.texto("Exportar CSV", exportar, 12.0, Peso::Normal, Alinear::Centro, TEXTO);
        self.zona(exportar, Accion::Exportar);

        let Some((_, historia)) = self.vista.historia.clone() else {
            l.texto("Cargando…", Caja::new(M, 100.0, INTERIOR, 24.0), 12.0, Peso::Normal, Alinear::Izquierda, TEXTO_2);
            return;
        };
        if historia.lecturas == 0 {
            l.parrafo(
                "Todavía no hay datos en este período. El historial se llena solo mientras la app esté abierta, aunque el panel esté cerrado.",
                Caja::new(M, 100.0, INTERIOR, 70.0),
                13.0,
                TEXTO_2,
            );
            self.pintar_pie(l, ALTO_DATOS - 34.0);
            return;
        }

        // Cifra principal y dos datos de apoyo
        l.texto(&duracion(historia.segundos_encendido), Caja::new(M, 88.0, 190.0, 38.0), 28.0, Peso::Liviano, Alinear::Izquierda, TEXTO);
        l.texto("prendido en este período", Caja::new(M, 124.0, 190.0, 16.0), 11.0, Peso::Normal, Alinear::Izquierda, TEXTO_3);
        let kwh = historia.segundos_enfriando_o_calentando as f32 / 3600.0 * self.vista.potencia as f32 / 1000.0;
        let x = ANCHO - M - 158.0;
        l.texto(&historia.encendidos.to_string(), Caja::new(x, 96.0, 60.0, 28.0), 18.0, Peso::Normal, Alinear::Izquierda, TEXTO);
        l.texto("encendidos", Caja::new(x, 124.0, 66.0, 16.0), 11.0, Peso::Normal, Alinear::Izquierda, TEXTO_3);
        l.texto(&format!("≈ {} kWh", decimal(kwh)), Caja::new(x + 68.0, 96.0, 92.0, 28.0), 18.0, Peso::Normal, Alinear::Izquierda, TEXTO);
        l.texto("estimado", Caja::new(x + 68.0, 124.0, 90.0, 16.0), 11.0, Peso::Normal, Alinear::Izquierda, TEXTO_3);

        self.pintar_grafico(l, &historia);

        // Tabla de temperaturas: los valores que el gráfico no rotula
        let y = 338.0;
        let columnas = [M + 66.0, M + 156.0, M + 246.0];
        for (i, nombre) in ["Promedio", "Mínima", "Máxima"].iter().enumerate() {
            l.texto(nombre, Caja::new(columnas[i], y, 80.0, 16.0), 11.0, Peso::Normal, Alinear::Izquierda, TEXTO_3);
        }
        for (f, (nombre, r)) in [("Adentro", historia.interior), ("Afuera", historia.exterior)].iter().enumerate() {
            let fy = y + 18.0 + f as f32 * 20.0;
            l.texto(nombre, Caja::new(M, fy, 66.0, 18.0), 12.0, Peso::Normal, Alinear::Izquierda, TEXTO_2);
            for (i, v) in [r.promedio, r.minimo, r.maximo].iter().enumerate() {
                let redondeado = v.map(|v| (v * 10.0).round() / 10.0);
                l.texto(&grados(redondeado), Caja::new(columnas[i], fy, 80.0, 18.0), 12.0, Peso::Normal, Alinear::Izquierda, TEXTO);
            }
        }

        // Tiempo por modo: una sola serie, cada barra lleva su nombre y su valor
        let mut y = 402.0;
        let mayor = historia.segundos_por_modo.first().map(|m| m.1).unwrap_or(1).max(1);
        for (modo, segundos) in historia.segundos_por_modo.iter().take(3) {
            let nombre = MODOS.iter().find(|m| m.0 == *modo).map(|m| m.1).unwrap_or("Otro");
            l.texto(nombre, Caja::new(M, y, 66.0, 18.0), 12.0, Peso::Normal, Alinear::Izquierda, TEXTO_2);
            let base = Caja::new(M + 66.0, y + 5.0, INTERIOR - 66.0 - 86.0, 8.0);
            let largo = (base.w * *segundos as f32 / mayor as f32).max(4.0);
            l.caja(Caja::new(base.x, base.y, largo, base.h), 4.0, LINEA_ADENTRO);
            l.texto(&duracion(*segundos), Caja::new(ANCHO - M - 82.0, y, 82.0, 18.0), 12.0, Peso::Normal, Alinear::Derecha, TEXTO);
            y += 20.0;
        }

        // Últimos eventos
        let mut y = 468.0;
        l.linea(M, y - 4.0, ANCHO - M, y - 4.0, 1.0, RAYA);
        for evento in historia.eventos.iter().take(5) {
            let cuando = if self.rango == 0 { hora(evento.ts) } else { dia_y_hora(evento.ts) };
            l.texto(&cuando, Caja::new(M, y, 74.0, 22.0), 11.0, Peso::Normal, Alinear::Izquierda, TEXTO_3);
            l.texto_recortado(&evento.texto, Caja::new(M + 76.0, y, INTERIOR - 76.0, 22.0), 11.5, TEXTO_2);
            y += 22.0;
        }
        self.pintar_pie(l, ALTO_DATOS - 34.0);
    }

    fn pintar_grafico(&mut self, l: &mut Lienzo, h: &Historia) {
        // Leyenda: una línea corta para cada serie
        let y_leyenda = 150.0;
        let mut x = M;
        for (nombre, color, grosor) in [
            ("Adentro", LINEA_ADENTRO, 2.0),
            ("Afuera", LINEA_AFUERA, 2.0),
            ("Objetivo", LINEA_OBJETIVO, 1.0),
        ] {
            l.linea(x, y_leyenda + 8.0, x + 14.0, y_leyenda + 8.0, grosor, color);
            let ancho = l.medir(nombre, 11.0, Peso::Normal);
            l.texto(nombre, Caja::new(x + 18.0, y_leyenda, ancho + 4.0, 16.0), 11.0, Peso::Normal, Alinear::Izquierda, TEXTO_2);
            x += 18.0 + ancho + 14.0;
        }
        l.caja(Caja::new(x, y_leyenda + 5.0, 14.0, 5.0), 2.0, Color(0xFFFFFF, 0.32));
        l.texto("Prendido", Caja::new(x + 18.0, y_leyenda, 60.0, 16.0), 11.0, Peso::Normal, Alinear::Izquierda, TEXTO_2);

        let p = GRAFICO;
        let valores = h.puntos.iter().flat_map(|q| [q.interior, q.exterior, q.objetivo]).flatten();
        let (mut menor, mut mayor) = (f32::MAX, f32::MIN);
        for v in valores {
            menor = menor.min(v);
            mayor = mayor.max(v);
        }
        if menor > mayor {
            return;
        }
        let paso = [1.0, 2.0, 5.0, 10.0].into_iter().find(|s| (mayor - menor + 1.0) / s <= 4.0).unwrap_or(10.0);
        let piso = ((menor - 0.5) / paso).floor() * paso;
        let techo = ((mayor + 0.5) / paso).ceil() * paso;
        let x_de = |ts: i64| p.x + (ts - h.desde) as f32 / (h.hasta - h.desde).max(1) as f32 * p.w;
        let y_de = |v: f32| p.y + p.h - (v - piso) / (techo - piso).max(1.0) * p.h;

        let mut valor = piso;
        while valor <= techo + 0.01 {
            let y = y_de(valor);
            l.linea(p.x, y, p.x + p.w, y, 1.0, Color(0xFFFFFF, 0.08));
            l.texto(&format!("{valor:.0}°"), Caja::new(M, y - 8.0, 24.0, 16.0), 10.5, Peso::Normal, Alinear::Izquierda, TEXTO_3);
            valor += paso;
        }

        // Tiras de "prendido" debajo del gráfico: el tiempo se lee sin mezclarse con las líneas
        let y_tira = p.y + p.h + 5.0;
        l.caja(Caja::new(p.x, y_tira, p.w, 5.0), 2.0, Color(0xFFFFFF, 0.05));
        let ancho_tramo = (p.w / 96.0).max(1.0);
        let mut i = 0;
        while i < h.puntos.len() {
            if h.puntos[i].encendido {
                let inicio = i;
                while i + 1 < h.puntos.len() && h.puntos[i + 1].encendido {
                    i += 1;
                }
                let x1 = x_de(h.puntos[inicio].ts) - ancho_tramo / 2.0;
                let x2 = x_de(h.puntos[i].ts) + ancho_tramo / 2.0;
                l.caja(Caja::new(x1, y_tira, (x2 - x1).max(2.0), 5.0), 2.0, Color(0xFFFFFF, 0.32));
            }
            i += 1;
        }

        // Un hueco largo corta la línea: no se inventan datos cuando la PC estuvo apagada
        let hueco = ((h.hasta - h.desde) / 96 * 3).max(1);
        let series: [(fn(&crate::historial::Punto) -> Option<f32>, Color, f32); 3] = [
            (|q| q.objetivo, LINEA_OBJETIVO, 1.0),
            (|q| q.exterior, LINEA_AFUERA, 2.0),
            (|q| q.interior, LINEA_ADENTRO, 2.0),
        ];
        for (leer, color, grosor) in series {
            let mut tramo: Vec<(f32, f32)> = Vec::new();
            let mut ultimo = None;
            for q in &h.puntos {
                let Some(v) = leer(q) else { continue };
                if ultimo.is_some_and(|t| q.ts - t > hueco) {
                    let _ = l.trazo(&tramo, grosor, color);
                    tramo.clear();
                }
                tramo.push((x_de(q.ts), y_de(v)));
                ultimo = Some(q.ts);
            }
            let _ = l.trazo(&tramo, grosor, color);
            if grosor > 1.0 {
                if let Some(&(x, y)) = tramo.last() {
                    l.circulo(x, y, 6.0, SUPERFICIE);
                    l.circulo(x, y, 4.0, color);
                }
            }
        }

        // Fechas abajo
        let y_fechas = y_tira + 7.0;
        for i in 0..4 {
            let ts = h.desde + (h.hasta - h.desde) * i as i64 / 3;
            let texto = if self.rango == 0 { hora(ts) } else { dia(ts) };
            let x = x_de(ts);
            let (caja, alinear) = match i {
                0 => (Caja::new(x, y_fechas, 60.0, 14.0), Alinear::Izquierda),
                3 => (Caja::new(x - 60.0, y_fechas, 60.0, 14.0), Alinear::Derecha),
                _ => (Caja::new(x - 30.0, y_fechas, 60.0, 14.0), Alinear::Centro),
            };
            l.texto(&texto, caja, 10.5, Peso::Normal, alinear, TEXTO_3);
        }

        // Cursor: busca la fecha más cercana y muestra todas las series
        let (rx, ry) = self.raton;
        if self.caliente.is_none() && p.adentro(-4.0).contiene(rx, ry) {
            if let Some(q) = h.puntos.iter().min_by_key(|q| (x_de(q.ts) - rx).abs() as i32) {
                let x = x_de(q.ts);
                l.linea(x, p.y, x, p.y + p.h, 1.0, Color(0xFFFFFF, 0.4));
                let filas = [("Adentro", q.interior, LINEA_ADENTRO), ("Afuera", q.exterior, LINEA_AFUERA), ("Objetivo", q.objetivo, LINEA_OBJETIVO)];
                let (ancho, alto) = (124.0, 84.0);
                let tx = if x + ancho + 10.0 > ANCHO - M { x - ancho - 8.0 } else { x + 8.0 };
                let globo = Caja::new(tx, p.y, ancho, alto);
                l.caja(globo, 6.0, Color::rgb(0x232D3A));
                l.borde(globo, 6.0, 1.0, RAYA);
                let titulo = if self.rango == 0 { hora(q.ts) } else { dia_y_hora(q.ts) };
                l.texto(&titulo, Caja::new(tx + 8.0, p.y + 4.0, ancho - 16.0, 16.0), 10.5, Peso::Normal, Alinear::Izquierda, TEXTO_3);
                for (i, (nombre, valor, color)) in filas.iter().enumerate() {
                    let fy = p.y + 21.0 + i as f32 * 15.0;
                    l.linea(tx + 8.0, fy + 8.0, tx + 17.0, fy + 8.0, 2.0, *color);
                    l.texto(&grados(valor.map(|v| (v * 2.0).round() / 2.0)), Caja::new(tx + 22.0, fy, 40.0, 16.0), 11.0, Peso::Medio, Alinear::Izquierda, TEXTO);
                    l.texto(nombre, Caja::new(tx + 62.0, fy, 56.0, 16.0), 10.5, Peso::Normal, Alinear::Izquierda, TEXTO_3);
                }
                let estado = if q.encendido { "Prendido" } else { "Apagado" };
                l.texto(estado, Caja::new(tx + 8.0, p.y + 66.0, ancho - 16.0, 14.0), 10.5, Peso::Normal, Alinear::Izquierda, TEXTO_3);
            }
        }
    }

    fn pintar_ajustes(&mut self, l: &mut Lienzo) {
        let mut y = 54.0;
        let fila = |y: f32| Caja::new(M, y, INTERIOR, 44.0);

        // Elegir solo hace falta si la cuenta tiene más de un aire
        if self.vista.aires.len() > 1 {
            let nombre = self.vista.aires.iter().find(|a| Some(a.id) == self.vista.aire_actual).map(|a| a.nombre.clone());
            let c = fila(y);
            l.caja(c, 8.0, self.fondo_de(Accion::SiguienteAire));
            l.texto("Aire", Caja::new(c.x + 12.0, c.y, 36.0, c.h), 13.0, Peso::Normal, Alinear::Izquierda, TEXTO);
            l.texto_recortado(&nombre.unwrap_or_default(), Caja::new(c.x + 48.0, c.y, c.w - 124.0, c.h), 13.0, TEXTO_2);
            l.texto("Cambiar", Caja::new(c.x + c.w - 72.0, c.y, 60.0, c.h), 12.0, Peso::Normal, Alinear::Derecha, TEXTO);
            self.zona(c, Accion::SiguienteAire);
            y += 50.0;
        }

        // Iniciar con Windows
        let c = fila(y);
        l.caja(c, 8.0, self.fondo_de(Accion::InicioWindows));
        l.texto("Iniciar con Windows", Caja::new(c.x + 12.0, c.y, 220.0, c.h), 13.0, Peso::Normal, Alinear::Izquierda, TEXTO);
        let interruptor = Caja::new(c.x + c.w - 52.0, c.y + 12.0, 40.0, 20.0);
        if self.inicio_windows {
            l.caja(interruptor, 10.0, Color::rgb(0x79C6F5));
            l.circulo(interruptor.x + 30.0, interruptor.y + 10.0, 7.0, SOBRE_ACENTO);
        } else {
            l.borde(interruptor, 10.0, 1.5, TEXTO_2);
            l.circulo(interruptor.x + 10.0, interruptor.y + 10.0, 5.5, TEXTO_2);
        }
        self.zona(c, Accion::InicioWindows);
        y += 50.0;

        // Potencia para estimar el consumo
        let c = fila(y);
        l.caja(c, 8.0, CONTROL);
        l.texto("Potencia para estimar consumo", Caja::new(c.x + 12.0, c.y, 200.0, c.h), 13.0, Peso::Normal, Alinear::Izquierda, TEXTO);
        self.boton_icono(l, Caja::new(c.x + c.w - 118.0, c.y + 8.0, 28.0, 28.0), ICONO_MENOS, 10.0, Accion::PotenciaMenos, self.vista.potencia > 100);
        l.texto(&format!("{} W", self.vista.potencia), Caja::new(c.x + c.w - 90.0, c.y, 56.0, c.h), 12.0, Peso::Normal, Alinear::Centro, TEXTO);
        self.boton_icono(l, Caja::new(c.x + c.w - 36.0, c.y + 8.0, 28.0, 28.0), ICONO_MAS, 10.0, Accion::PotenciaMas, self.vista.potencia < 10_000);
        y += 50.0;

        let desde = self.vista.primera_lectura.map(dia).unwrap_or_else(|| "hoy".to_string());
        l.parrafo(
            &format!("{} lecturas guardadas desde {desde}.", self.vista.lecturas_totales),
            Caja::new(M, y + 2.0, INTERIOR, 18.0),
            11.5,
            TEXTO_3,
        );
        y += 26.0;

        let ancho = (INTERIOR - 8.0) / 2.0;
        for (i, (nombre, accion)) in [("Exportar CSV", Accion::Exportar), ("Abrir carpeta de datos", Accion::AbrirCarpeta)].into_iter().enumerate() {
            let c = Caja::new(M + i as f32 * (ancho + 8.0), y, ancho, 38.0);
            l.caja(c, 8.0, self.fondo_de(accion));
            l.texto(nombre, c, 12.0, Peso::Normal, Alinear::Centro, TEXTO);
            self.zona(c, accion);
        }
        y += 56.0;

        l.linea(M, y - 8.0, ANCHO - M, y - 8.0, 1.0, RAYA);
        for (i, (nombre, accion)) in [("Cerrar sesión", Accion::CerrarSesion), ("Salir del programa", Accion::Salir)].into_iter().enumerate() {
            let c = Caja::new(M + i as f32 * (ancho + 8.0), y, ancho, 38.0);
            l.caja(c, 8.0, self.fondo_de(accion));
            l.texto(nombre, c, 12.0, Peso::Normal, Alinear::Centro, TEXTO);
            self.zona(c, accion);
        }
    }

    fn pintar_login(&mut self, l: &mut Lienzo) {
        l.texto("Entrá con tu cuenta de Carrier", Caja::new(M, 16.0, INTERIOR, 30.0), 16.0, Peso::Medio, Alinear::Izquierda, TEXTO);
        l.texto("Es la misma de la app del aire.", Caja::new(M, 44.0, INTERIOR, 20.0), 12.0, Peso::Normal, Alinear::Izquierda, TEXTO_2);

        for (titulo, y) in [("Email", Y_CAMPO_CUENTA), ("Contraseña", Y_CAMPO_CLAVE)] {
            l.texto(titulo, Caja::new(M, y - 20.0, INTERIOR, 18.0), 11.0, Peso::Normal, Alinear::Izquierda, TEXTO_2);
            l.caja(Caja::new(M, y, INTERIOR, 36.0), 6.0, Color::rgb(CAMPO));
        }

        let y = Y_CAMPO_CLAVE + 46.0;
        let marca = Caja::new(M, y + 2.0, 20.0, 20.0);
        if self.recordar {
            l.caja(marca, 4.0, Color::rgb(0x79C6F5));
            l.icono(ICONO_MARCA, marca, 11.0, SOBRE_ACENTO);
        } else {
            l.borde(marca.adentro(0.75), 4.0, 1.5, TEXTO_2);
        }
        l.texto("Recordar en esta PC", Caja::new(M + 28.0, y, INTERIOR - 28.0, 24.0), 13.0, Peso::Normal, Alinear::Izquierda, TEXTO);
        self.zona(Caja::new(M, y, INTERIOR, 24.0), Accion::Recordar);
        l.texto(
            "Se guarda en el Administrador de credenciales de Windows.",
            Caja::new(M, y + 24.0, INTERIOR, 18.0),
            11.0,
            Peso::Normal,
            Alinear::Izquierda,
            TEXTO_3,
        );

        let boton = Caja::new(M, y + 50.0, INTERIOR, 42.0);
        let alfa = if self.apretado == Some(Accion::Entrar) { 0.75 } else { 1.0 };
        l.caja(boton, 8.0, Color(0x79C6F5, alfa));
        l.texto("Entrar", boton, 14.0, Peso::Medio, Alinear::Centro, SOBRE_ACENTO);
        self.zona(boton, Accion::Entrar);

        if let Some(error) = self.vista.error.clone() {
            l.parrafo(&error, Caja::new(M, y + 100.0, INTERIOR, 40.0), 12.0, ERROR);
        }
    }

    fn pintar_espera(&mut self, l: &mut Lienzo) {
        l.texto("Aire del cuarto", Caja::new(M, 16.0, INTERIOR, 30.0), 15.0, Peso::Medio, Alinear::Izquierda, TEXTO);
        match self.vista.error.clone() {
            Some(error) => {
                l.parrafo(&error, Caja::new(M, 60.0, INTERIOR, 40.0), 13.0, ERROR);
                l.parrafo("Se sigue intentando sola.", Caja::new(M, 100.0, INTERIOR, 20.0), 12.0, TEXTO_2);
                let c = Caja::new(M, 130.0, 150.0, 34.0);
                l.caja(c, 8.0, self.fondo_de(Accion::CerrarSesion));
                l.texto("Usar otra cuenta", c, 12.0, Peso::Normal, Alinear::Centro, TEXTO);
                self.zona(c, Accion::CerrarSesion);
                if self.vista.aires.len() > 1 {
                    let c = Caja::new(M + 158.0, 130.0, 150.0, 34.0);
                    l.caja(c, 8.0, self.fondo_de(Accion::SiguienteAire));
                    l.texto("Probar otro aire", c, 12.0, Peso::Normal, Alinear::Centro, TEXTO);
                    self.zona(c, Accion::SiguienteAire);
                }
            }
            None => l.parrafo("Conectando con tu aire…", Caja::new(M, 60.0, INTERIOR, 40.0), 13.0, TEXTO_2),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum VistaPanel {
    Login,
    Espera,
    Normal,
}

const REGLA_X: f32 = M + 6.0;
const REGLA_ANCHO: f32 = INTERIOR - 12.0;
const REGLA_Y: f32 = 166.0;
const GRAFICO: Caja = Caja::new(M + 26.0, 172.0, INTERIOR - 26.0, 118.0);
const Y_CAMPO_CUENTA: f32 = 92.0;
const Y_CAMPO_CLAVE: f32 = 152.0;

fn dibujar_copo(l: &mut Lienzo, x: f32, y: f32, radio: f32, color: Color) {
    use std::f32::consts::PI;
    for k in 0..6 {
        let a = k as f32 * PI / 3.0 - PI / 2.0;
        let (dx, dy) = (a.cos(), a.sin());
        l.linea(x, y, x + dx * radio, y + dy * radio, 1.3, color);
        let (bx, by) = (x + dx * radio * 0.6, y + dy * radio * 0.6);
        for lado in [-1.0f32, 1.0] {
            let b = a + lado * PI / 3.0;
            l.linea(bx, by, bx + b.cos() * radio * 0.32, by + b.sin() * radio * 0.32, 1.3, color);
        }
    }
}

/// Tres líneas de viento: rectas que terminan en una curva hacia arriba o abajo.
fn dibujar_viento(l: &mut Lienzo, x: f32, y: f32, color: Color) {
    use std::f32::consts::PI;
    for (dy, largo, radio, abajo) in [(-5.0f32, 11.0f32, 2.6f32, false), (0.0, 15.0, 3.0, false), (5.0, 8.0, 2.6, true)] {
        let inicio = x - 9.0;
        let fin = inicio + largo;
        let mut puntos = vec![(inicio, y + dy), (fin, y + dy)];
        let signo = if abajo { 1.0 } else { -1.0 };
        for paso in 1..=8 {
            let a = paso as f32 / 8.0 * PI * 1.25;
            puntos.push((fin + a.sin() * radio, y + dy + signo * (radio - a.cos() * radio)));
        }
        let _ = l.trazo(&puntos, 1.3, color);
    }
}

fn coordenadas(app: &App, lparam: LPARAM) -> (f32, f32) {
    let escala = app.escala();
    let x = (lparam.0 & 0xFFFF) as i16 as f32 / escala;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32 / escala;
    (x, y)
}

unsafe extern "system" fn procedimiento(ventana: HWND, mensaje: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let puntero = unsafe { GetWindowLongPtrW(ventana, GWLP_USERDATA) } as *mut App;
    if puntero.is_null() {
        return unsafe { DefWindowProcW(ventana, mensaje, wparam, lparam) };
    }
    let app = unsafe { &mut *puntero };
    match mensaje {
        WM_NCCALCSIZE if wparam.0 != 0 => LRESULT(0),
        WM_NCHITTEST => LRESULT(HTCLIENT as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut datos = PAINTSTRUCT::default();
            unsafe {
                BeginPaint(ventana, &mut datos);
            }
            app.pintar();
            unsafe {
                let _ = EndPaint(ventana, &datos);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            if let Some(l) = &app.lienzo {
                let dpi = unsafe { GetDpiForWindow(ventana) } as f32;
                let _ = l.ajustar((lparam.0 & 0xFFFF) as u32, ((lparam.0 >> 16) & 0xFFFF) as u32, dpi);
            }
            app.redibujar();
            LRESULT(0)
        }
        WM_DPICHANGED => {
            app.lienzo = None;
            app.crear_fuente_campos();
            app.ubicar();
            app.sincronizar_campos();
            LRESULT(0)
        }
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                app.ocultar();
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = coordenadas(app, lparam);
            app.al_mover(x, y);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            app.siguiendo_raton = false;
            if !app.opciones.sin_ocultar {
                app.caliente = None;
                app.raton = (-1.0, -1.0);
                app.redibujar();
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = coordenadas(app, lparam);
            app.al_apretar(x, y);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = coordenadas(app, lparam);
            app.al_soltar(x, y);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let giro = ((wparam.0 >> 16) & 0xFFFF) as i16;
            if app.pestana == Pestana::Control {
                app.mover_objetivo(if giro > 0 { 1.0 } else { -1.0 });
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let cursor = if app.caliente.is_some() { IDC_HAND } else { IDC_ARROW };
            // Los campos de texto usan su propio cursor
            if HWND(wparam.0 as *mut _) != ventana {
                return unsafe { DefWindowProcW(ventana, mensaje, wparam, lparam) };
            }
            unsafe {
                if let Ok(c) = LoadCursorW(None, cursor) {
                    SetCursor(Some(c));
                }
            }
            LRESULT(1)
        }
        WM_KEYDOWN => {
            app.al_tecla(wparam.0 as u16);
            LRESULT(0)
        }
        WM_CTLCOLOREDIT => {
            let dc = HDC(wparam.0 as *mut _);
            unsafe {
                SetTextColor(dc, hexa_color(TEXTO.0));
                SetBkColor(dc, hexa_color(CAMPO));
            }
            LRESULT(app.pincel_campo.0 as isize)
        }
        WM_TIMER => {
            match wparam.0 {
                TEMPORIZADOR_ENVIO => app.enviar_pendientes(),
                TEMPORIZADOR_RELOJ => {
                    if app.vista.apagar_en.is_some() || app.vista.encender_en.is_some() {
                        app.redibujar();
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        MENSAJE_VISTA => {
            app.al_recibir_vista();
            LRESULT(0)
        }
        MENSAJE_BANDEJA => {
            match (lparam.0 & 0xFFFF) as u32 {
                WM_LBUTTONUP => app.alternar(),
                WM_RBUTTONUP => app.menu_bandeja(),
                _ => {}
            }
            LRESULT(0)
        }
        MENSAJE_ABRIR => {
            app.mostrar();
            LRESULT(0)
        }
        WM_CLOSE => {
            app.salir();
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        otro if otro == MENSAJE_BARRA.load(Ordering::Relaxed) && otro != 0 => {
            // El explorador se reinició: hay que volver a poner el ícono
            if let Some((_, _, icono)) = &app.icono {
                bandeja::agregar(ventana, *icono, &app.tooltip);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(ventana, mensaje, wparam, lparam) },
    }
}

pub const MENSAJE_ABRIR: u32 = WM_APP + 3;
pub const CLASE: PCWSTR = w!("AireControlPanel");

pub fn crear_ventana(titulo: &str) -> windows::core::Result<HWND> {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    unsafe {
        let modulo = GetModuleHandleW(None)?;
        let clase = WNDCLASSW {
            lpfnWndProc: Some(procedimiento),
            hInstance: modulo.into(),
            lpszClassName: CLASE,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&clase);
        MENSAJE_BARRA.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        let titulo_w: Vec<u16> = titulo.encode_utf16().chain(Some(0)).collect();
        let ventana = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            CLASE,
            PCWSTR(titulo_w.as_ptr()),
            WS_POPUP | WS_THICKFRAME | WS_CLIPCHILDREN,
            0,
            0,
            380,
            524,
            None,
            None,
            Some(modulo.into()),
            None,
        )?;

        let oscuro: i32 = 1;
        let _ = DwmSetWindowAttribute(ventana, DWMWA_USE_IMMERSIVE_DARK_MODE, &oscuro as *const _ as *const _, 4);
        let esquinas = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(ventana, DWMWA_WINDOW_CORNER_PREFERENCE, &esquinas as *const _ as *const _, 4);

        for (campo, nombre) in [(&CAMPO_CUENTA, w!("Email")), (&CAMPO_CLAVE, w!("Contraseña"))] {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                PCWSTR::null(),
                WS_CHILD | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                0,
                0,
                100,
                22,
                Some(ventana),
                None,
                Some(modulo.into()),
                None,
            )?;
            let _ = SetWindowTextW(hwnd, PCWSTR::null());
            let _ = nombre;
            campo.store(hwnd.0 as isize, Ordering::Relaxed);
        }
        Ok(ventana)
    }
}

/// Atiende Enter, Tab y Esc en los campos de texto. Devuelve true si consumió la tecla.
pub fn tecla_de_campo(mensaje: &MSG, ventana: HWND) -> bool {
    if mensaje.message != WM_KEYDOWN {
        return false;
    }
    let cuenta = HWND(CAMPO_CUENTA.load(Ordering::Relaxed) as *mut _);
    let clave = HWND(CAMPO_CLAVE.load(Ordering::Relaxed) as *mut _);
    if mensaje.hwnd != cuenta && mensaje.hwnd != clave {
        return false;
    }
    let puntero = unsafe { GetWindowLongPtrW(ventana, GWLP_USERDATA) } as *mut App;
    if puntero.is_null() {
        return false;
    }
    let app = unsafe { &mut *puntero };
    match VIRTUAL_KEY(mensaje.wParam.0 as u16) {
        VK_RETURN => {
            if mensaje.hwnd == cuenta {
                unsafe {
                    let _ = SetFocus(Some(clave));
                }
            } else {
                app.entrar();
            }
            true
        }
        VK_TAB => {
            unsafe {
                let _ = SetFocus(Some(if mensaje.hwnd == cuenta { clave } else { cuenta }));
            }
            true
        }
        VK_ESCAPE => {
            app.ocultar();
            true
        }
        _ => false,
    }
}

pub fn instalar(ventana: HWND, pedidos: Sender<Pedido>, compartido: Compartido, opciones: Opciones) {
    let abrir = opciones.abrir;
    let pestana = opciones.pestana;
    let app = Box::new(App {
        ventana,
        pedidos,
        compartido,
        opciones,
        fabricas: None,
        lienzo: None,
        vista: Vista::default(),
        tooltip: "Aire del cuarto".to_string(),
        icono: None,
        zonas: Vec::new(),
        caliente: None,
        apretado: None,
        foco: None,
        teclado: false,
        raton: (-1.0, -1.0),
        siguiendo_raton: false,
        pestana,
        rango: 0,
        pendientes: Cambios::default(),
        en_vuelo: Cambios::default(),
        numero_cambio: 0,
        visible: false,
        oculto_en: None,
        recordar: true,
        con_vidrio: false,
        memoria_inicial_devuelta: false,
        inicio_windows: inicio::activo(),
        pincel_campo: unsafe { CreateSolidBrush(hexa_color(CAMPO)) },
        fuente_campo: HFONT::default(),
    });
    let puntero = Box::into_raw(app);
    unsafe {
        SetWindowLongPtrW(ventana, GWLP_USERDATA, puntero as isize);
        (*puntero).crear_fuente_campos();
        (*puntero).actualizar_icono();
        if abrir {
            let _ = PostMessageW(Some(ventana), MENSAJE_ABRIR, WPARAM(0), LPARAM(0));
        }
    }
}
