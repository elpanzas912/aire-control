//! Hash y AES con la criptografía de Windows, para no sumar dependencias.
use windows::Win32::Security::Cryptography::{
    BCryptDecrypt, BCryptDestroyKey, BCryptEncrypt, BCryptGenerateSymmetricKey, BCryptHash,
    BCRYPT_AES_CBC_ALG_HANDLE, BCRYPT_AES_ECB_ALG_HANDLE, BCRYPT_ALG_HANDLE, BCRYPT_BLOCK_PADDING,
    BCRYPT_KEY_HANDLE, BCRYPT_MD5_ALG_HANDLE, BCRYPT_SHA256_ALG_HANDLE,
};

pub fn md5(datos: &[u8]) -> [u8; 16] {
    let mut salida = [0u8; 16];
    let estado = unsafe { BCryptHash(BCRYPT_MD5_ALG_HANDLE, None, datos, &mut salida) };
    assert!(estado.is_ok(), "MD5 falló: {estado:?}");
    salida
}

pub fn sha256(datos: &[u8]) -> [u8; 32] {
    let mut salida = [0u8; 32];
    let estado = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, datos, &mut salida) };
    assert!(estado.is_ok(), "SHA-256 falló: {estado:?}");
    salida
}

pub fn hexa(datos: &[u8]) -> String {
    datos.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn de_hexa(texto: &str) -> Option<Vec<u8>> {
    if texto.len() % 2 != 0 {
        return None;
    }
    (0..texto.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(texto.get(i..i + 2)?, 16).ok())
        .collect()
}

struct Clave(BCRYPT_KEY_HANDLE);

impl Clave {
    fn nueva(algoritmo: BCRYPT_ALG_HANDLE, secreto: &[u8]) -> Option<Self> {
        let mut clave = BCRYPT_KEY_HANDLE::default();
        let estado =
            unsafe { BCryptGenerateSymmetricKey(algoritmo, &mut clave, None, secreto, 0) };
        estado.is_ok().then_some(Self(clave))
    }
}

impl Drop for Clave {
    fn drop(&mut self) {
        unsafe {
            let _ = BCryptDestroyKey(self.0);
        }
    }
}

fn cifrar(algoritmo: BCRYPT_ALG_HANDLE, secreto: &[u8], iv: Option<&[u8]>, datos: &[u8]) -> Option<Vec<u8>> {
    let clave = Clave::nueva(algoritmo, secreto)?;
    let mut iv = iv.map(|v| v.to_vec());
    let mut salida = vec![0u8; datos.len() + 16];
    let mut escritos = 0u32;
    let estado = unsafe {
        BCryptEncrypt(
            clave.0,
            Some(datos),
            None,
            iv.as_deref_mut(),
            Some(&mut salida),
            &mut escritos,
            BCRYPT_BLOCK_PADDING,
        )
    };
    if !estado.is_ok() {
        return None;
    }
    salida.truncate(escritos as usize);
    Some(salida)
}

fn descifrar(algoritmo: BCRYPT_ALG_HANDLE, secreto: &[u8], iv: Option<&[u8]>, datos: &[u8]) -> Option<Vec<u8>> {
    let clave = Clave::nueva(algoritmo, secreto)?;
    let mut iv = iv.map(|v| v.to_vec());
    let mut salida = vec![0u8; datos.len()];
    let mut escritos = 0u32;
    let estado = unsafe {
        BCryptDecrypt(
            clave.0,
            Some(datos),
            None,
            iv.as_deref_mut(),
            Some(&mut salida),
            &mut escritos,
            BCRYPT_BLOCK_PADDING,
        )
    };
    if !estado.is_ok() {
        return None;
    }
    salida.truncate(escritos as usize);
    Some(salida)
}

pub fn cifrar_cbc(clave: &str, iv: &str, texto: &str) -> Option<String> {
    cifrar(BCRYPT_AES_CBC_ALG_HANDLE, clave.as_bytes(), Some(iv.as_bytes()), texto.as_bytes())
        .map(|d| hexa(&d))
}

pub fn descifrar_cbc(clave: &str, iv: &str, datos_hexa: &str) -> Option<String> {
    let datos = de_hexa(datos_hexa)?;
    let plano = descifrar(BCRYPT_AES_CBC_ALG_HANDLE, clave.as_bytes(), Some(iv.as_bytes()), &datos)?;
    String::from_utf8(plano).ok()
}

#[cfg(test)]
pub fn cifrar_ecb(clave: &[u8], datos: &[u8]) -> Option<Vec<u8>> {
    cifrar(BCRYPT_AES_ECB_ALG_HANDLE, clave, None, datos)
}

pub fn descifrar_ecb(clave: &[u8], datos: &[u8]) -> Option<Vec<u8>> {
    descifrar(BCRYPT_AES_ECB_ALG_HANDLE, clave, None, datos)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn cbc_coincide_con_python() {
        let cifrado = cifrar_cbc("0123456789abcdef", "fedcba9876543210", "hola,-1,2").unwrap();
        assert_eq!(cifrado, "5e643d729c9f80c206ccfa611176c6e7");
        assert_eq!(
            descifrar_cbc("0123456789abcdef", "fedcba9876543210", &cifrado).unwrap(),
            "hola,-1,2"
        );
    }

    #[test]
    fn ecb_ida_y_vuelta() {
        let clave = md5(b"clave");
        let cifrado = cifrar_ecb(&clave, b"texto de prueba largo").unwrap();
        assert_eq!(descifrar_ecb(&clave, &cifrado).unwrap(), b"texto de prueba largo");
    }

    #[test]
    fn hexa_ida_y_vuelta() {
        assert_eq!(de_hexa(&hexa(&[0, 15, 255])).unwrap(), [0, 15, 255]);
        assert!(de_hexa("abc").is_none());
        assert!(de_hexa("zz").is_none());
    }
}
