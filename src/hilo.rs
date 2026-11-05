//! Hilo de fondo: duerme hasta la próxima tarea, así no gasta CPU entre lecturas.
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::credenciales;
use crate::inicio;
use crate::motor::{Aviso, Motor, Vista};
use crate::nube::{Nube, Transporte};
use crate::protocolo::Cambios;
use crate::tiempo::ahora;

pub const MENSAJE_VISTA: u32 = WM_APP + 1;

pub enum Pedido {
    Conectar { cuenta: String, contrasena: String, recordar: bool },
    Olvidar,
    SiguienteAire,
    Actualizar,
    Cambiar(Cambios, u64),
    SumarApagado,
    SumarEncendido,
    CancelarApagado,
    CancelarEncendido,
    PanelAbierto(bool),
    Historia(u8),
    Potencia(i64),
    /// En centavos
    PrecioKwh(i64),
    ExportarCsv,
    Salir,
}

#[derive(Default)]
pub struct Publicado {
    pub vista: Vista,
    pub avisos: Vec<Aviso>,
    pub tooltip: String,
}

pub type Compartido = Arc<Mutex<Publicado>>;

pub struct Inicio {
    pub motor: Motor,
    pub ventana: isize,
    pub compartido: Compartido,
    pub transporte_inicial: Option<Box<dyn Transporte>>,
}

fn exportar(motor: &mut Motor) {
    let ruta = inicio::carpeta_documentos().join("AireControl-historial.csv");
    let resultado = std::fs::File::create(&ruta)
        .map_err(|e| e.to_string())
        .and_then(|archivo| motor.exportar_csv(&mut std::io::BufWriter::new(archivo)));
    match resultado {
        Ok(filas) => {
            motor.avisos.push(Aviso {
                titulo: "Historial exportado".into(),
                texto: format!("{filas} filas en Documentos."),
            });
            let _ = std::process::Command::new("explorer")
                .arg(format!("/select,{}", ruta.display()))
                .spawn();
        }
        Err(error) => motor.avisos.push(Aviso {
            titulo: "No se pudo exportar".into(),
            texto: error,
        }),
    }
}

pub fn correr(mut inicio: Inicio, pedidos: Receiver<Pedido>) {
    let ventana = HWND(inicio.ventana as *mut _);
    let motor = &mut inicio.motor;
    let mut pidiendo_recordar: Option<(String, String)> = None;
    let mut vigilia = false;

    if let Some(transporte) = inicio.transporte_inicial.take() {
        let _ = motor.conectar(transporte, ahora());
    }

    loop {
        // Mientras corre un temporizador la PC no se suspende, para que pueda cumplirse.
        // La pantalla sí puede apagarse.
        let quiere_vigilia = motor.hay_temporizadores();
        if quiere_vigilia != vigilia {
            unsafe {
                SetThreadExecutionState(if quiere_vigilia {
                    ES_CONTINUOUS | ES_SYSTEM_REQUIRED
                } else {
                    ES_CONTINUOUS
                });
            }
            vigilia = quiere_vigilia;
        }

        {
            let mut publicado = inicio.compartido.lock().unwrap();
            publicado.vista = motor.vista.clone();
            publicado.avisos.append(&mut motor.avisos);
            publicado.tooltip = motor.resumen_tooltip(ahora());
        }
        unsafe {
            let _ = PostMessageW(Some(ventana), MENSAJE_VISTA, WPARAM(0), LPARAM(0));
        }

        let espera = motor.segundos_hasta_tarea(ahora()).max(0) as u64;
        let pedido = match pedidos.recv_timeout(Duration::from_secs(espera)) {
            Ok(p) => Some(p),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let t = ahora();
        match pedido {
            Some(Pedido::Salir) => break,
            Some(Pedido::Conectar { cuenta, contrasena, recordar }) => {
                let nube = Nube::nueva(&cuenta, &contrasena);
                if motor.conectar(Box::new(nube), t).is_ok() && recordar {
                    credenciales::guardar_cuenta(&cuenta, &contrasena);
                } else if recordar {
                    // Sin internet todavía no se sabe si la cuenta es válida: se guarda cuando conecte
                    pidiendo_recordar = Some((cuenta, contrasena));
                }
            }
            Some(Pedido::Olvidar) => {
                credenciales::borrar_cuenta();
                pidiendo_recordar = None;
                motor.olvidar_cuenta();
            }
            Some(Pedido::SiguienteAire) => motor.siguiente_aire(t),
            Some(Pedido::Actualizar) => motor.sondear(t),
            Some(Pedido::Cambiar(cambios, numero)) => motor.cambiar(cambios, numero, t),
            Some(Pedido::SumarApagado) => motor.sumar_apagado(t),
            Some(Pedido::SumarEncendido) => motor.sumar_encendido(t),
            Some(Pedido::CancelarApagado) => motor.cancelar_apagado(t),
            Some(Pedido::CancelarEncendido) => motor.cancelar_encendido(t),
            Some(Pedido::PanelAbierto(abierto)) => motor.panel_abierto(abierto, t),
            Some(Pedido::Historia(rango)) => motor.pedir_historia(rango, t),
            Some(Pedido::Potencia(watts)) => motor.fijar_potencia(watts),
            Some(Pedido::PrecioKwh(centavos)) => motor.fijar_precio(centavos),
            Some(Pedido::ExportarCsv) => exportar(motor),
            None => {}
        }
        motor.tic(ahora());
        if motor.vista.fase == crate::motor::Fase::Conectado {
            if let Some((cuenta, contrasena)) = pidiendo_recordar.take() {
                credenciales::guardar_cuenta(&cuenta, &contrasena);
            }
        }
    }
    unsafe {
        SetThreadExecutionState(ES_CONTINUOUS);
    }
}
