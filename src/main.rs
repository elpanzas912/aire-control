#![cfg_attr(not(test), windows_subsystem = "windows")]

mod animacion;
mod bandeja;
mod cifrado;
mod credenciales;
#[cfg(feature = "demo")]
mod demo;
mod dinero;
mod hilo;
mod historial;
mod inicio;
mod lienzo;
mod motor;
mod nube;
mod panel;
mod protocolo;
mod red;
mod tiempo;

use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, LPARAM, WPARAM};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, FindWindowW, GetMessageW, MessageBoxW, PostMessageW, TranslateMessage, MB_ICONERROR, MB_OK, MSG,
};

use historial::Historial;
use panel::{Opciones, Pestana};

fn avisar_error(texto: &str) {
    let ancho: Vec<u16> = texto.encode_utf16().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(None, PCWSTR(ancho.as_ptr()), w!("Aire del cuarto"), MB_OK | MB_ICONERROR);
    }
}

fn main() {
    let argumentos: Vec<String> = std::env::args().collect();
    let hay = |bandera: &str| argumentos.iter().any(|a| a == bandera);
    let demo = cfg!(feature = "demo") && hay("--demo");
    let titulo = if demo { "Aire (demostración)" } else { "Aire del cuarto" };

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    // Una sola copia: si ya está abierta, se abre su panel
    let nombre_mutex = if demo { w!("AireControlDemo") } else { w!("AireControlUnico") };
    let _unico = unsafe { CreateMutexW(None, false, nombre_mutex) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            if let Ok(existente) = FindWindowW(panel::CLASE, PCWSTR::null()) {
                let _ = PostMessageW(Some(existente), panel::MENSAJE_ABRIR, WPARAM(0), LPARAM(0));
            }
        }
        return;
    }

    let ruta = if demo {
        std::env::temp_dir().join("aire-control-demo.db")
    } else {
        inicio::carpeta_datos().join("aire.db")
    };
    if demo {
        for sufijo in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{sufijo}", ruta.display()));
        }
    }
    let registro = match Historial::abrir(&ruta) {
        Ok(h) => h,
        Err(error) => {
            avisar_error(&format!("No se pudo abrir el historial en {}: {error}", ruta.display()));
            return;
        }
    };

    #[cfg(feature = "demo")]
    if demo {
        demo::sembrar(&registro, tiempo::ahora());
    }

    let motor = motor::Motor::nuevo(registro, tiempo::ahora());

    let ventana = match panel::crear_ventana(titulo) {
        Ok(v) => v,
        Err(error) => {
            avisar_error(&format!("No se pudo crear la ventana: {error}"));
            return;
        }
    };

    let transporte_inicial: Option<Box<dyn nube::Transporte>> = if demo {
        // La demostración nunca usa la cuenta real
        #[cfg(feature = "demo")]
        let simulado = (!hay("--login")).then(|| Box::new(demo::AireDemo::nuevo()) as Box<dyn nube::Transporte>);
        #[cfg(not(feature = "demo"))]
        let simulado = None;
        simulado
    } else {
        credenciales::leer_cuenta().map(|(cuenta, clave)| {
            Box::new(nube::Nube::nueva(&cuenta, &clave)) as Box<dyn nube::Transporte>
        })
    };
    let hay_cuenta = transporte_inicial.is_some();

    let (envio, recepcion) = channel();
    let compartido: hilo::Compartido = Arc::new(Mutex::new(hilo::Publicado::default()));
    let pestana = if hay("--datos") {
        Pestana::Datos
    } else if hay("--ajustes") {
        Pestana::Ajustes
    } else {
        Pestana::Control
    };
    panel::instalar(
        ventana,
        envio,
        compartido.clone(),
        Opciones {
            sin_ocultar: demo && hay("--fijo"),
            pestana,
            // Al iniciar con Windows queda en la bandeja; al abrirlo a mano se muestra,
            // igual que si no hay cuenta y hace falta entrar
            abrir: !hay("--bandeja") || !hay_cuenta,
            titulo: if demo { "Aire (demostración)" } else { "Aire del cuarto" },
            // Solo para revisar las animaciones en la demostración
            lentitud: if demo && hay("--lento") { 8.0 } else { 1.0 },
            quieto: demo && hay("--quieto"),
            hay_cuenta,
        },
    );

    let inicio_hilo = hilo::Inicio {
        motor,
        ventana: ventana.0 as isize,
        compartido,
        transporte_inicial,
    };
    std::thread::spawn(move || hilo::correr(inicio_hilo, recepcion));

    let mut mensaje = MSG::default();
    unsafe {
        while GetMessageW(&mut mensaje, None, 0, 0).as_bool() {
            if panel::tecla_de_campo(&mensaje, ventana) {
                continue;
            }
            let _ = TranslateMessage(&mensaje);
            DispatchMessageW(&mensaje);
        }
    }
    // No se espera al hilo: un pedido en curso a la nube podría demorar el cierre hasta 20 segundos.
    // SQLite deja el historial consistente aunque el proceso termine a mitad de una escritura.
}
