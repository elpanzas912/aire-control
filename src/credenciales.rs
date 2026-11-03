//! Cuenta guardada en el Administrador de credenciales de Windows.
//! Usa las mismas entradas que el programa en Python, así no hace falta volver a entrar.
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

const SERVICIO: &str = "AireControlPc";
const USUARIO_CUENTA: &str = "cuenta";

fn ancho(texto: &str) -> Vec<u16> {
    texto.encode_utf16().chain(Some(0)).collect()
}

struct Leida {
    usuario: String,
    valor: String,
}

fn leer(destino: &str) -> Option<Leida> {
    let destino = ancho(destino);
    let mut puntero: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe {
        CredReadW(PCWSTR(destino.as_ptr()), CRED_TYPE_GENERIC, None, &mut puntero).ok()?;
        let c = &*puntero;
        let unidades = std::slice::from_raw_parts(
            c.CredentialBlob as *const u16,
            c.CredentialBlobSize as usize / 2,
        );
        let leida = Leida {
            usuario: c.UserName.to_string().unwrap_or_default(),
            valor: String::from_utf16_lossy(unidades),
        };
        CredFree(puntero as *const _);
        Some(leida)
    }
}

fn escribir(destino: &str, usuario: &str, valor: &str) -> bool {
    let mut destino = ancho(destino);
    let mut usuario = ancho(usuario);
    let mut bytes: Vec<u8> = valor.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let credencial = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(destino.as_mut_ptr()),
        CredentialBlobSize: bytes.len() as u32,
        CredentialBlob: bytes.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(usuario.as_mut_ptr()),
        ..Default::default()
    };
    unsafe { CredWriteW(&credencial, 0).is_ok() }
}

fn borrar_destino(destino: &str) {
    let destino = ancho(destino);
    unsafe {
        let _ = CredDeleteW(PCWSTR(destino.as_ptr()), CRED_TYPE_GENERIC, None);
    }
}

fn destino_de(cuenta: &str) -> String {
    format!("{cuenta}@{SERVICIO}")
}

pub fn leer_cuenta() -> Option<(String, String)> {
    let base = leer(SERVICIO)?;
    if base.usuario != USUARIO_CUENTA || base.valor.is_empty() {
        return None;
    }
    let contrasena = leer(&destino_de(&base.valor))?.valor;
    (!contrasena.is_empty()).then_some((base.valor, contrasena))
}

pub fn guardar_cuenta(cuenta: &str, contrasena: &str) -> bool {
    escribir(&destino_de(cuenta), cuenta, contrasena) && escribir(SERVICIO, USUARIO_CUENTA, cuenta)
}

pub fn borrar_cuenta() {
    if let Some(base) = leer(SERVICIO) {
        if base.usuario == USUARIO_CUENTA {
            borrar_destino(&destino_de(&base.valor));
        }
    }
    borrar_destino(SERVICIO);
}
