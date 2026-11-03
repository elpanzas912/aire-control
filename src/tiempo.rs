//! Fechas y duraciones. Internamente todo son segundos Unix; lo local es solo para mostrar.
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

pub fn ahora() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn local(ts: i64) -> SYSTEMTIME {
    let intervalos = (ts + 11_644_473_600) as u64 * 10_000_000;
    let archivo = FILETIME {
        dwLowDateTime: intervalos as u32,
        dwHighDateTime: (intervalos >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut salida = SYSTEMTIME::default();
    unsafe {
        let _ = FileTimeToSystemTime(&archivo, &mut utc);
        let _ = SystemTimeToTzSpecificLocalTime(None, &utc, &mut salida);
    }
    salida
}

pub fn hora(ts: i64) -> String {
    let t = local(ts);
    format!("{:02}:{:02}", t.wHour, t.wMinute)
}

pub fn dia_y_hora(ts: i64) -> String {
    let t = local(ts);
    format!("{}/{} {:02}:{:02}", t.wDay, t.wMonth, t.wHour, t.wMinute)
}

pub fn dia(ts: i64) -> String {
    let t = local(ts);
    format!("{}/{}", t.wDay, t.wMonth)
}

pub fn fecha_iso(ts: i64) -> String {
    let t = local(ts);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

/// Marca local que pide el servidor en cada pedido (la app oficial usa hora local, no UTC).
pub fn marca_pedido() -> String {
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

/// Marca en UTC que va en cada paquete: centésimas, segundos, ... año.
pub fn marca_paquete() -> [u8; 8] {
    let t = unsafe { GetSystemTime() };
    [
        (t.wMilliseconds / 10) as u8,
        t.wSecond as u8,
        t.wMinute as u8,
        t.wHour as u8,
        t.wDay as u8,
        t.wMonth as u8,
        (t.wYear % 100) as u8,
        (t.wYear / 100) as u8,
    ]
}

/// "45 min", "1 h 30 min", "2 h". Redondea hacia arriba: nunca promete menos tiempo del real.
pub fn duracion(segundos: i64) -> String {
    let minutos = (segundos.max(0) + 59) / 60;
    let (h, m) = (minutos / 60, minutos % 60);
    match (h, m) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn duraciones() {
        assert_eq!(duracion(0), "0 min");
        assert_eq!(duracion(1), "1 min");
        assert_eq!(duracion(30 * 60), "30 min");
        assert_eq!(duracion(90 * 60), "1 h 30 min");
        assert_eq!(duracion(120 * 60), "2 h");
        assert_eq!(duracion(-5), "0 min");
    }

    #[test]
    fn marcas_tienen_formato() {
        assert_eq!(marca_pedido().len(), 14);
        assert_eq!(hora(ahora()).len(), 5);
    }
}
