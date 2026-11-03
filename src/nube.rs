//! Sesión en la nube de Midea. El servidor ya no entrega la clave local (getToken da 9999), pero
//! reenvía comandos al aire. Identidad y campos son los de Midea Air, según jasonzzeng/midea-streamdeck.
use std::collections::BTreeMap;

use crate::cifrado::{cifrar_cbc, descifrar_cbc, hexa, md5, sha256};
use crate::protocolo::{a_lista, armar_paquete, de_lista, extraer_trama};
use crate::red::Cliente;
use crate::tiempo::{marca_paquete, marca_pedido};

const APP_ID: &str = "1117";
const APP_KEY: &str = "ff0cf6f5f0c3471de36341cab3f7a9af";
const SESION_INVALIDA: &str = "3106";
const TODAVIA_BLOQUEADA: &str = "7610";

#[derive(Clone, Debug, PartialEq)]
pub enum ErrorNube {
    /// La cuenta fue rechazada: no se reintenta solo, porque Midea puede bloquearla.
    Cuenta { codigo: String },
    /// La cuenta entró, pero no tiene aires o Midea no entrega la lista: tampoco se reintenta solo.
    Aires { codigo: Option<String> },
    /// La sesión venció o la tomó otro programa con esta identidad.
    Sesion,
    /// Midea respondió con otro error, por ejemplo 3176 si el aire no contesta.
    Servidor { codigo: String },
    Red(String),
    Formato(String),
}

impl ErrorNube {
    pub fn mensaje(&self) -> String {
        match self {
            Self::Cuenta { codigo } if codigo == TODAVIA_BLOQUEADA => {
                "Midea bloqueó la cuenta por muchos intentos. Esperá unos minutos.".into()
            }
            Self::Cuenta { .. } => "Midea rechazó el email o la contraseña.".into(),
            Self::Aires { codigo: None } => {
                "Esta cuenta no tiene ningún aire vinculado. Vinculalo desde la app de Carrier.".into()
            }
            Self::Aires { codigo: Some(codigo) } => format!("Midea no entregó la lista de aires (error {codigo})."),
            Self::Sesion => "Otro programa está usando esta cuenta. Cerralo y actualizá.".into(),
            Self::Servidor { codigo } if codigo == "3176" => {
                "El aire no respondió. Fijate que tenga luz y WiFi.".into()
            }
            Self::Servidor { codigo } => format!("Midea respondió con el error {codigo}."),
            Self::Red(_) => "No se pudo conectar con Midea. Revisá tu internet.".into(),
            Self::Formato(_) => "Midea envió una respuesta que no se entiende.".into(),
        }
    }
}

/// Un aire vinculado a la cuenta.
#[derive(Clone, Debug, PartialEq)]
pub struct Aire {
    pub id: u64,
    pub nombre: String,
}

/// Lo único que el resto del programa necesita de la nube. Las pruebas usan un aire simulado.
pub trait Transporte: Send {
    fn iniciar_sesion(&mut self) -> Result<(), ErrorNube>;
    fn enviar(&mut self, trama: &[u8]) -> Result<Vec<u8>, ErrorNube>;

    fn aires(&self) -> Vec<Aire> {
        Vec::new()
    }

    fn usar_aire(&mut self, _id: u64) {}
}

struct Sesion {
    id: String,
    clave: String,
    iv: String,
}

pub struct Nube {
    cuenta: String,
    contrasena: String,
    aires: Vec<Aire>,
    aire_id: Option<u64>,
    cliente: Option<Cliente>,
    sesion: Option<Sesion>,
    terminal: String,
}

fn firma(ruta: &str, campos: &BTreeMap<&str, String>) -> String {
    let consulta: Vec<String> = campos.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let texto = format!("{ruta}{}{APP_KEY}", consulta.join("&"));
    hexa(&sha256(texto.as_bytes()))
}

fn codificar(texto: &str) -> String {
    let mut salida = String::with_capacity(texto.len());
    for b in texto.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => salida.push(b as char),
            _ => salida.push_str(&format!("%{b:02X}")),
        }
    }
    salida
}

fn hash_contrasena(login_id: &str, contrasena: &str) -> String {
    let interno = hexa(&sha256(contrasena.as_bytes()));
    hexa(&sha256(format!("{login_id}{interno}{APP_KEY}").as_bytes()))
}

fn hash_contrasena_iam(login_id: &str, contrasena: &str) -> String {
    let doble = hexa(&md5(hexa(&md5(contrasena.as_bytes())).as_bytes()));
    hexa(&sha256(format!("{login_id}{doble}{APP_KEY}").as_bytes()))
}

fn clave_de_app() -> (String, String) {
    let h = hexa(&sha256(APP_KEY.as_bytes()));
    (h[..16].to_string(), h[16..32].to_string())
}

/// Se queda con los aires: la cuenta puede tener otros aparatos. El número puede venir como texto.
fn aires_de(datos: &serde_json::Value) -> Vec<Aire> {
    let Some(lista) = datos["list"].as_array() else {
        return Vec::new();
    };
    lista
        .iter()
        .filter(|a| a["type"].as_str().is_some_and(|t| t.eq_ignore_ascii_case("0xAC")))
        .filter_map(|a| match &a["id"] {
            serde_json::Value::String(id) => id.parse().ok().map(|id| (id, a)),
            serde_json::Value::Number(id) => id.as_u64().map(|id| (id, a)),
            _ => None,
        })
        .enumerate()
        .map(|(i, (id, a))| {
            let nombre = a["name"].as_str().unwrap_or_default().trim();
            let nombre = if nombre.is_empty() { format!("Aire {}", i + 1) } else { nombre.to_string() };
            Aire { id, nombre }
        })
        .collect()
}

impl Nube {
    pub fn nueva(cuenta: &str, contrasena: &str) -> Self {
        Self {
            cuenta: cuenta.to_string(),
            contrasena: contrasena.to_string(),
            aires: Vec::new(),
            aire_id: None,
            cliente: None,
            sesion: None,
            terminal: hexa(&md5(format!("aire-pc:{cuenta}").as_bytes())),
        }
    }

    fn campos_base() -> BTreeMap<&'static str, String> {
        BTreeMap::from([
            ("format", "2".to_string()),
            ("language", "en-US".to_string()),
            ("src", "17".to_string()),
            ("stamp", marca_pedido()),
        ])
    }

    fn campos_login(&self) -> BTreeMap<&'static str, String> {
        let mut campos = Self::campos_base();
        campos.insert("appId", APP_ID.to_string());
        campos.insert("clientType", "2".to_string());
        campos.insert("encryptVersion", "1".to_string());
        campos.insert("phoneModel", "iOS".to_string());
        campos.insert("phoneSysVNum", "18.7.8".to_string());
        campos.insert("terminalId", self.terminal.clone());
        campos.insert("loginAccount", self.cuenta.clone());
        campos
    }

    fn pedir(&mut self, ruta: &str, mut campos: BTreeMap<&str, String>) -> Result<serde_json::Value, ErrorNube> {
        let sign = firma(ruta, &campos);
        campos.insert("sign", sign);
        let cuerpo: Vec<String> = campos.iter().map(|(k, v)| format!("{k}={}", codificar(v))).collect();
        if self.cliente.is_none() {
            self.cliente = Some(Cliente::nuevo().map_err(ErrorNube::Red)?);
        }
        let cliente = self.cliente.as_ref().ok_or_else(|| ErrorNube::Red("sin cliente".into()))?;
        let texto = cliente.enviar_formulario(ruta, &cuerpo.join("&")).map_err(ErrorNube::Red)?;
        let respuesta: serde_json::Value =
            serde_json::from_str(&texto).map_err(|e| ErrorNube::Formato(e.to_string()))?;
        let codigo = match &respuesta["errorCode"] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => "0".to_string(),
        };
        match codigo.as_str() {
            "0" => Ok(respuesta["result"].clone()),
            SESION_INVALIDA => Err(ErrorNube::Sesion),
            _ => Err(ErrorNube::Servidor { codigo }),
        }
    }

    fn buscar_aires(&mut self) -> Result<Vec<Aire>, ErrorNube> {
        let sesion = self.sesion.as_ref().ok_or(ErrorNube::Sesion)?;
        let mut campos = Self::campos_base();
        campos.insert("sessionId", sesion.id.clone());
        let datos = self.pedir("/v1/appliance/user/list/get", campos).map_err(|e| match e {
            ErrorNube::Servidor { codigo } => ErrorNube::Aires { codigo: Some(codigo) },
            otro => otro,
        })?;
        let aires = aires_de(&datos);
        if aires.is_empty() {
            return Err(ErrorNube::Aires { codigo: None });
        }
        Ok(aires)
    }
}

fn texto(valor: &serde_json::Value, campo: &str) -> Result<String, ErrorNube> {
    valor[campo]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ErrorNube::Formato(format!("falta {campo}")))
}

impl Transporte for Nube {
    fn iniciar_sesion(&mut self) -> Result<(), ErrorNube> {
        self.sesion = None;
        let como_cuenta = |e| match e {
            ErrorNube::Servidor { codigo } => ErrorNube::Cuenta { codigo },
            otro => otro,
        };
        let id = self
            .pedir("/v1/user/login/id/get", self.campos_login())
            .map_err(como_cuenta)?;
        let login_id = texto(&id, "loginId")?;
        let mut campos = self.campos_login();
        campos.insert("password", hash_contrasena(&login_id, &self.contrasena));
        campos.insert("iampwd", hash_contrasena_iam(&login_id, &self.contrasena));
        let datos = self.pedir("/v1/user/login/new", campos).map_err(como_cuenta)?;

        let (clave, iv) = clave_de_app();
        let abrir = |campo: &str| -> Result<String, ErrorNube> {
            descifrar_cbc(&clave, &iv, &texto(&datos, campo)?)
                .ok_or_else(|| ErrorNube::Formato(format!("no se pudo abrir {campo}")))
        };
        self.sesion = Some(Sesion {
            id: texto(&datos, "sessionId")?,
            clave: abrir("accessToken")?,
            iv: abrir("randomData")?,
        });
        if self.aires.is_empty() {
            self.aires = self.buscar_aires()?;
            self.aire_id = self.aires.first().map(|a| a.id);
        }
        Ok(())
    }

    fn enviar(&mut self, trama: &[u8]) -> Result<Vec<u8>, ErrorNube> {
        let sesion = self.sesion.as_ref().ok_or(ErrorNube::Sesion)?;
        let aire_id = self.aire_id.ok_or(ErrorNube::Sesion)?;
        let (id, clave, iv) = (sesion.id.clone(), sesion.clave.clone(), sesion.iv.clone());
        let paquete = armar_paquete(aire_id, trama, marca_paquete());
        let orden = cifrar_cbc(&clave, &iv, &a_lista(&paquete))
            .ok_or_else(|| ErrorNube::Formato("no se pudo cifrar".into()))?;
        let mut campos = Self::campos_base();
        campos.insert("applianceId", aire_id.to_string());
        campos.insert("funId", "FC02".to_string());
        campos.insert("order", orden);
        campos.insert("sessionId", id);
        let datos = self.pedir("/v1/appliance/transparent/send/new", campos)?;
        let respuesta = texto(&datos, "reply").map_err(|_| ErrorNube::Servidor { codigo: "3176".into() })?;
        descifrar_cbc(&clave, &iv, &respuesta)
            .and_then(|plano| de_lista(&plano))
            .and_then(|bytes| extraer_trama(&bytes))
            .ok_or_else(|| ErrorNube::Formato("respuesta ilegible".into()))
    }

    fn aires(&self) -> Vec<Aire> {
        self.aires.clone()
    }

    fn usar_aire(&mut self, id: u64) {
        self.aire_id = Some(id);
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn firma_coincide_con_python() {
        let campos = BTreeMap::from([
            ("appId", "1117".to_string()),
            ("stamp", "20261103120000".to_string()),
            ("loginAccount", "yo@correo.com".to_string()),
            ("format", "2".to_string()),
        ]);
        assert_eq!(
            firma("/v1/user/login/new", &campos),
            "fdb10c9b3141e16de87158f8a52081d32821ba29770f3c7dfb8f856532f5c203"
        );
    }

    #[test]
    fn hashes_de_contrasena_coinciden_con_python() {
        let (login, clave) = ("abcdef0123456789", "contrasena-de-prueba");
        assert_eq!(
            hash_contrasena(login, clave),
            "2b4dac9e8e78fd94c58c6b7020810135982ed8206781263c635893a7c40d34ec"
        );
        assert_eq!(
            hash_contrasena_iam(login, clave),
            "f959138b39a917294057a756b260249b46ab6753b96cc75e53e71b9c4ca194fb"
        );
    }

    #[test]
    fn clave_de_app_abre_el_token_de_python() {
        let (clave, iv) = clave_de_app();
        assert_eq!(format!("{clave}{iv}"), &"303f7223403a0163ac5919cb8b208df5fde1744fea27d7aa014613099edc9c31"[..32]);
        let token = "82fc448be9b5233dbe32940923b6ea0d7c7b60a0a7d0f8d8bc81623576f089f9";
        assert_eq!(descifrar_cbc(&clave, &iv, token).unwrap(), "0123456789abcdef");
    }

    #[test]
    fn codifica_el_formulario() {
        assert_eq!(codificar("yo+x@correo.com"), "yo%2Bx%40correo.com");
        assert_eq!(codificar("a-b_c.d~1"), "a-b_c.d~1");
        assert_eq!(codificar("ñ"), "%C3%B1");
    }

    #[test]
    fn mensajes_para_la_persona() {
        assert!(ErrorNube::Cuenta { codigo: "3102".into() }.mensaje().contains("rechazó"));
        assert!(ErrorNube::Cuenta { codigo: "7610".into() }.mensaje().contains("minutos"));
        assert!(ErrorNube::Servidor { codigo: "3176".into() }.mensaje().contains("no respondió"));
        assert!(ErrorNube::Sesion.mensaje().contains("Otro programa"));
        assert!(ErrorNube::Aires { codigo: None }.mensaje().contains("ningún aire"));
        assert!(ErrorNube::Aires { codigo: Some("1234".into()) }.mensaje().contains("1234"));
    }

    #[test]
    fn de_la_lista_solo_toma_los_aires() {
        let datos = serde_json::json!({"list": [
            {"id": "1234", "name": " Cuarto ", "type": "0xAC"},
            {"id": 5678, "name": "", "type": "0xac"},
            {"id": "9", "name": "Heladera", "type": "0xCA"},
            {"id": "sin número", "name": "Roto", "type": "0xAC"},
        ]});
        assert_eq!(
            aires_de(&datos),
            vec![
                Aire { id: 1234, nombre: "Cuarto".into() },
                Aire { id: 5678, nombre: "Aire 2".into() },
            ]
        );
        assert!(aires_de(&serde_json::json!({})).is_empty());
    }
}
