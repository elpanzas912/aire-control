//! "Iniciar con Windows" y carpetas del programa. Lo activa la persona desde el panel o el menú.
use std::path::PathBuf;

use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

const CLAVE_RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALOR: PCWSTR = w!("AireControl");

pub fn activo() -> bool {
    let mut largo = 0u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, CLAVE_RUN, VALOR, RRF_RT_REG_SZ, None, None, Some(&mut largo)).is_ok()
    }
}

pub fn fijar(activar: bool) -> bool {
    unsafe {
        if !activar {
            return RegDeleteKeyValueW(HKEY_CURRENT_USER, CLAVE_RUN, VALOR).is_ok();
        }
        let Ok(programa) = std::env::current_exe() else { return false };
        let comando = format!("\"{}\" --bandeja", programa.display());
        let datos: Vec<u16> = comando.encode_utf16().chain(Some(0)).collect();
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            CLAVE_RUN,
            VALOR,
            REG_SZ.0,
            Some(datos.as_ptr() as *const _),
            (datos.len() * 2) as u32,
        )
        .is_ok()
    }
}

pub fn carpeta_datos() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let carpeta = base.join("AireControl");
    let _ = std::fs::create_dir_all(&carpeta);
    carpeta
}

pub fn carpeta_documentos() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(|p| PathBuf::from(p).join("Documents"))
        .filter(|p| p.is_dir())
        .unwrap_or_else(carpeta_datos)
}
