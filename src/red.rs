//! POST por HTTPS con WinHTTP: usa el TLS de Windows y no suma peso al programa.
use std::ffi::c_void;

use windows::core::{w, PCWSTR};
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
    WinHttpQueryDataAvailable, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

const SERVIDOR: PCWSTR = w!("mapp.appsmb.com");
const PUERTO_HTTPS: u16 = 443;
const ESPERA_MS: i32 = 20_000;

struct Manija(*mut c_void);

impl Drop for Manija {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

pub struct Cliente {
    conexion: Manija,
    _sesion: Manija,
}

// Las manijas de WinHTTP se pueden usar desde cualquier hilo
unsafe impl Send for Cliente {}

fn error_de_sistema(que: &str) -> String {
    format!("{que}: {}", windows::core::Error::from_thread())
}

impl Cliente {
    pub fn nuevo() -> Result<Self, String> {
        unsafe {
            let sesion = Manija(WinHttpOpen(
                w!("Midea OBM/5.20.0 (iPhone; iOS 18.7.8; Scale/3.00)"),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ));
            if sesion.0.is_null() {
                return Err(error_de_sistema("WinHttpOpen"));
            }
            let _ = WinHttpSetTimeouts(sesion.0, ESPERA_MS, ESPERA_MS, ESPERA_MS, ESPERA_MS);
            let conexion = Manija(WinHttpConnect(sesion.0, SERVIDOR, PUERTO_HTTPS, 0));
            if conexion.0.is_null() {
                return Err(error_de_sistema("WinHttpConnect"));
            }
            Ok(Self { conexion, _sesion: sesion })
        }
    }

    pub fn enviar_formulario(&self, ruta: &str, cuerpo: &str) -> Result<String, String> {
        let ruta_w: Vec<u16> = ruta.encode_utf16().chain(Some(0)).collect();
        let cabeceras: Vec<u16> = "Content-Type: application/x-www-form-urlencoded\r\n"
            .encode_utf16()
            .collect();
        unsafe {
            let pedido = Manija(WinHttpOpenRequest(
                self.conexion.0,
                w!("POST"),
                PCWSTR(ruta_w.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_OPEN_REQUEST_FLAGS(WINHTTP_FLAG_SECURE.0),
            ));
            if pedido.0.is_null() {
                return Err(error_de_sistema("WinHttpOpenRequest"));
            }
            WinHttpSendRequest(
                pedido.0,
                Some(&cabeceras),
                Some(cuerpo.as_ptr() as *const c_void),
                cuerpo.len() as u32,
                cuerpo.len() as u32,
                0,
            )
            .map_err(|e| format!("WinHttpSendRequest: {e}"))?;
            WinHttpReceiveResponse(pedido.0, std::ptr::null_mut())
                .map_err(|e| format!("WinHttpReceiveResponse: {e}"))?;

            let mut codigo = 0u32;
            let mut largo = 4u32;
            WinHttpQueryHeaders(
                pedido.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut codigo as *mut u32 as *mut c_void),
                &mut largo,
                std::ptr::null_mut(),
            )
            .map_err(|e| format!("WinHttpQueryHeaders: {e}"))?;
            if codigo != 200 {
                return Err(format!("HTTP {codigo}"));
            }

            let mut salida = Vec::new();
            loop {
                let mut disponibles = 0u32;
                WinHttpQueryDataAvailable(pedido.0, &mut disponibles)
                    .map_err(|e| format!("WinHttpQueryDataAvailable: {e}"))?;
                if disponibles == 0 {
                    break;
                }
                let inicio = salida.len();
                salida.resize(inicio + disponibles as usize, 0);
                let mut leidos = 0u32;
                WinHttpReadData(
                    pedido.0,
                    salida[inicio..].as_mut_ptr() as *mut c_void,
                    disponibles,
                    &mut leidos,
                )
                .map_err(|e| format!("WinHttpReadData: {e}"))?;
                salida.truncate(inicio + leidos as usize);
            }
            String::from_utf8(salida).map_err(|_| "Respuesta con texto inválido".to_string())
        }
    }
}
