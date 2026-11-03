//! Historial en SQLite: cada lectura del aire, los eventos y los ajustes del programa.
use rusqlite::{params, Connection, OptionalExtension};

use crate::protocolo::Estado;

/// Un hueco mayor a esto entre lecturas no se cuenta como tiempo encendido (PC apagada, sin internet).
const HUECO_MAXIMO: i64 = 15 * 60;

#[derive(Clone, Debug, PartialEq)]
pub struct Evento {
    pub ts: i64,
    pub texto: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Punto {
    pub ts: i64,
    pub interior: Option<f32>,
    pub exterior: Option<f32>,
    pub objetivo: Option<f32>,
    pub encendido: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Resumen {
    pub promedio: Option<f32>,
    pub minimo: Option<f32>,
    pub maximo: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Historia {
    pub desde: i64,
    pub hasta: i64,
    pub puntos: Vec<Punto>,
    pub segundos_encendido: i64,
    pub segundos_enfriando_o_calentando: i64,
    pub encendidos: u32,
    pub segundos_por_modo: Vec<(u8, i64)>,
    pub interior: Resumen,
    pub exterior: Resumen,
    pub eventos: Vec<Evento>,
    pub lecturas: u32,
}

pub struct Historial {
    db: Connection,
}

impl Historial {
    pub fn abrir(ruta: &std::path::Path) -> rusqlite::Result<Self> {
        Self::preparar(Connection::open(ruta)?)
    }

    #[cfg(test)]
    pub fn en_memoria() -> rusqlite::Result<Self> {
        Self::preparar(Connection::open_in_memory()?)
    }

    fn preparar(db: Connection) -> rusqlite::Result<Self> {
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-256;
             CREATE TABLE IF NOT EXISTS lecturas(
                 ts INTEGER PRIMARY KEY,
                 encendido INTEGER NOT NULL,
                 modo INTEGER NOT NULL,
                 objetivo REAL NOT NULL,
                 interior REAL,
                 exterior REAL,
                 ventilador INTEGER NOT NULL,
                 giro INTEGER NOT NULL,
                 eco INTEGER NOT NULL,
                 turbo INTEGER NOT NULL,
                 sueno INTEGER NOT NULL,
                 seguir INTEGER NOT NULL,
                 purificador INTEGER NOT NULL,
                 aux INTEGER NOT NULL,
                 pantalla INTEGER NOT NULL,
                 filtro INTEGER NOT NULL,
                 error INTEGER NOT NULL,
                 crudo BLOB
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS eventos(
                 id INTEGER PRIMARY KEY,
                 ts INTEGER NOT NULL,
                 texto TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS eventos_ts ON eventos(ts);
             CREATE TABLE IF NOT EXISTS ajustes(
                 clave TEXT PRIMARY KEY,
                 valor TEXT NOT NULL
             ) WITHOUT ROWID;",
        )?;
        Ok(Self { db })
    }

    pub fn guardar_lectura(&self, ts: i64, e: &Estado, crudo_cambio: bool) -> rusqlite::Result<()> {
        // El dato crudo solo se guarda si cambió: permite interpretar más campos en el futuro
        let crudo = crudo_cambio.then_some(e.crudo.as_slice());
        self.db
            .prepare_cached(
                "INSERT OR REPLACE INTO lecturas
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            )?
            .execute(params![
                ts,
                e.encendido,
                e.modo,
                e.objetivo,
                e.interior,
                e.exterior,
                e.ventilador,
                e.giro,
                e.eco,
                e.turbo,
                e.sueno,
                e.seguir,
                e.purificador,
                e.aux || e.aux_solo,
                e.pantalla,
                e.filtro,
                e.error,
                crudo,
            ])?;
        Ok(())
    }

    pub fn guardar_evento(&self, ts: i64, texto: &str) -> rusqlite::Result<()> {
        self.db
            .prepare_cached("INSERT INTO eventos(ts, texto) VALUES (?1, ?2)")?
            .execute(params![ts, texto])?;
        Ok(())
    }

    pub fn ajuste(&self, clave: &str) -> Option<String> {
        self.db
            .query_row("SELECT valor FROM ajustes WHERE clave=?1", [clave], |f| f.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn ajuste_numero(&self, clave: &str) -> Option<i64> {
        self.ajuste(clave)?.parse().ok()
    }

    pub fn fijar_ajuste(&self, clave: &str, valor: Option<String>) -> rusqlite::Result<()> {
        match valor {
            Some(v) => self.db.execute(
                "INSERT OR REPLACE INTO ajustes(clave, valor) VALUES (?1, ?2)",
                params![clave, v],
            )?,
            None => self.db.execute("DELETE FROM ajustes WHERE clave=?1", [clave])?,
        };
        Ok(())
    }

    pub fn total_lecturas(&self) -> (u64, Option<i64>) {
        self.db
            .query_row("SELECT COUNT(*), MIN(ts) FROM lecturas", [], |f| {
                Ok((f.get::<_, i64>(0)? as u64, f.get(1)?))
            })
            .unwrap_or((0, None))
    }

    fn resumen(&self, columna: &str, desde: i64, hasta: i64) -> rusqlite::Result<Resumen> {
        let sql = format!("SELECT AVG({columna}), MIN({columna}), MAX({columna}) FROM lecturas WHERE ts BETWEEN ?1 AND ?2");
        self.db.query_row(&sql, [desde, hasta], |f| {
            Ok(Resumen {
                promedio: f.get::<_, Option<f64>>(0)?.map(|v| v as f32),
                minimo: f.get::<_, Option<f64>>(1)?.map(|v| v as f32),
                maximo: f.get::<_, Option<f64>>(2)?.map(|v| v as f32),
            })
        })
    }

    pub fn historia(&self, desde: i64, hasta: i64, tramos: i64) -> rusqlite::Result<Historia> {
        let mut h = Historia {
            desde,
            hasta,
            interior: self.resumen("interior", desde, hasta)?,
            exterior: self.resumen("exterior", desde, hasta)?,
            ..Default::default()
        };
        let ancho = ((hasta - desde) / tramos.max(1)).max(1);
        let mut consulta = self.db.prepare_cached(
            "SELECT (ts-?1)/?3, AVG(interior), AVG(exterior), AVG(objetivo), MAX(encendido)
             FROM lecturas WHERE ts BETWEEN ?1 AND ?2 GROUP BY 1 ORDER BY 1",
        )?;
        h.puntos = consulta
            .query_map([desde, hasta, ancho], |f| {
                Ok(Punto {
                    ts: desde + f.get::<_, i64>(0)? * ancho + ancho / 2,
                    interior: f.get::<_, Option<f64>>(1)?.map(|v| v as f32),
                    exterior: f.get::<_, Option<f64>>(2)?.map(|v| v as f32),
                    objetivo: f.get::<_, Option<f64>>(3)?.map(|v| v as f32),
                    encendido: f.get::<_, i64>(4)? != 0,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        let mut filas = self.db.prepare_cached(
            "SELECT ts, encendido, modo FROM lecturas WHERE ts BETWEEN ?1 AND ?2 ORDER BY ts",
        )?;
        let mut anterior: Option<(i64, bool, u8)> = None;
        let mut por_modo = std::collections::BTreeMap::<u8, i64>::new();
        let mut contar = |h: &mut Historia, previa: (i64, bool, u8), fin: i64| {
            let tramo = (fin - previa.0).clamp(0, HUECO_MAXIMO);
            if previa.1 {
                h.segundos_encendido += tramo;
                *por_modo.entry(previa.2).or_default() += tramo;
                if previa.2 != crate::protocolo::MODO_VENTILAR {
                    h.segundos_enfriando_o_calentando += tramo;
                }
            }
        };
        let iterador = filas.query_map([desde, hasta], |f| Ok((f.get(0)?, f.get::<_, bool>(1)?, f.get::<_, u8>(2)?)))?;
        for fila in iterador {
            let actual: (i64, bool, u8) = fila?;
            h.lecturas += 1;
            if let Some(previa) = anterior {
                contar(&mut h, previa, actual.0);
                if actual.1 && !previa.1 {
                    h.encendidos += 1;
                }
            } else if actual.1 {
                h.encendidos += 1;
            }
            anterior = Some(actual);
        }
        if let Some(previa) = anterior {
            contar(&mut h, previa, hasta);
        }
        h.segundos_por_modo = por_modo.into_iter().collect();
        h.segundos_por_modo.sort_by_key(|(_, s)| -*s);

        let mut eventos = self.db.prepare_cached(
            "SELECT ts, texto FROM eventos WHERE ts BETWEEN ?1 AND ?2 ORDER BY ts DESC, id DESC LIMIT 5",
        )?;
        h.eventos = eventos
            .query_map([desde, hasta], |f| Ok(Evento { ts: f.get(0)?, texto: f.get(1)? }))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(h)
    }

    /// Exporta todas las lecturas y eventos, para abrirlos en una planilla.
    pub fn exportar_csv(&self, salida: &mut impl std::io::Write) -> Result<u64, String> {
        let error = |e: rusqlite::Error| e.to_string();
        writeln!(salida, "fecha_hora,encendido,modo,objetivo,interior,exterior,ventilador,giro,eco,turbo,sueno,seguir,purificador,aux,pantalla,filtro,error,evento")
            .map_err(|e| e.to_string())?;
        let mut consulta = self
            .db
            .prepare(
                "SELECT ts, encendido, modo, objetivo, interior, exterior, ventilador, giro, eco, turbo,
                        sueno, seguir, purificador, aux, pantalla, filtro, error, NULL
                 FROM lecturas
                 UNION ALL
                 SELECT ts, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
                        NULL, NULL, NULL, NULL, NULL, NULL, NULL, texto
                 FROM eventos
                 ORDER BY 1",
            )
            .map_err(error)?;
        let mut filas = consulta.query([]).map_err(error)?;
        let mut total = 0;
        while let Some(f) = filas.next().map_err(error)? {
            let mut partes = vec![crate::tiempo::fecha_iso(f.get(0).map_err(error)?)];
            for i in 1..17 {
                let celda = match f.get_ref(i).map_err(error)? {
                    rusqlite::types::ValueRef::Integer(v) => v.to_string(),
                    rusqlite::types::ValueRef::Real(v) => v.to_string(),
                    _ => String::new(),
                };
                partes.push(celda);
            }
            let evento: Option<String> = f.get(17).map_err(error)?;
            partes.push(evento.map(|t| format!("\"{}\"", t.replace('"', "\"\""))).unwrap_or_default());
            writeln!(salida, "{}", partes.join(",")).map_err(|e| e.to_string())?;
            total += 1;
        }
        Ok(total)
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn lectura(encendido: bool, interior: f32) -> Estado {
        Estado {
            encendido,
            objetivo: 22.0,
            modo: crate::protocolo::MODO_FRIO,
            ventilador: 102,
            interior: Some(interior),
            exterior: Some(30.0),
            pantalla: true,
            crudo: vec![0xC0],
            ..Default::default()
        }
    }

    #[test]
    fn cuenta_tiempo_encendido_y_encendidos() {
        let h = Historial::en_memoria().unwrap();
        h.guardar_lectura(1000, &lectura(false, 25.0), true).unwrap();
        h.guardar_lectura(1060, &lectura(true, 24.0), true).unwrap();
        h.guardar_lectura(1120, &lectura(true, 23.0), false).unwrap();
        h.guardar_lectura(1180, &lectura(false, 22.0), true).unwrap();
        h.guardar_lectura(1240, &lectura(true, 22.0), true).unwrap();
        let r = h.historia(900, 1300, 10).unwrap();
        assert_eq!(r.lecturas, 5);
        assert_eq!(r.encendidos, 2);
        assert_eq!(r.segundos_encendido, 120 + 60);
        assert_eq!(r.segundos_por_modo, vec![(crate::protocolo::MODO_FRIO, 180)]);
        assert_eq!(r.interior.minimo, Some(22.0));
        assert_eq!(r.interior.maximo, Some(25.0));
    }

    #[test]
    fn un_hueco_largo_no_cuenta_como_encendido() {
        let h = Historial::en_memoria().unwrap();
        h.guardar_lectura(0, &lectura(true, 24.0), true).unwrap();
        h.guardar_lectura(100_000, &lectura(true, 24.0), false).unwrap();
        let r = h.historia(0, 100_000, 10).unwrap();
        assert_eq!(r.segundos_encendido, HUECO_MAXIMO);
    }

    #[test]
    fn ventilar_no_suma_al_consumo_estimado() {
        let h = Historial::en_memoria().unwrap();
        let mut e = lectura(true, 24.0);
        e.modo = crate::protocolo::MODO_VENTILAR;
        h.guardar_lectura(0, &e, true).unwrap();
        h.guardar_lectura(60, &lectura(false, 24.0), true).unwrap();
        let r = h.historia(0, 60, 10).unwrap();
        assert_eq!(r.segundos_encendido, 60);
        assert_eq!(r.segundos_enfriando_o_calentando, 0);
    }

    #[test]
    fn agrupa_puntos_por_tramo() {
        let h = Historial::en_memoria().unwrap();
        for i in 0..100 {
            h.guardar_lectura(i * 60, &lectura(i % 2 == 0, 20.0 + (i % 2) as f32), false).unwrap();
        }
        let r = h.historia(0, 6000, 10).unwrap();
        assert_eq!(r.puntos.len(), 10);
        assert_eq!(r.puntos[0].interior, Some(20.5));
        assert!(r.puntos[0].encendido);
    }

    #[test]
    fn eventos_recientes_primero_y_limitados() {
        let h = Historial::en_memoria().unwrap();
        for i in 0..8 {
            h.guardar_evento(i, &format!("evento {i}")).unwrap();
        }
        let r = h.historia(0, 100, 10).unwrap();
        assert_eq!(r.eventos.len(), 5);
        assert_eq!(r.eventos[0].texto, "evento 7");
    }

    #[test]
    fn ajustes_se_guardan_y_borran() {
        let h = Historial::en_memoria().unwrap();
        assert_eq!(h.ajuste("x"), None);
        h.fijar_ajuste("x", Some("5".into())).unwrap();
        assert_eq!(h.ajuste_numero("x"), Some(5));
        h.fijar_ajuste("x", None).unwrap();
        assert_eq!(h.ajuste("x"), None);
    }

    #[test]
    fn exporta_lecturas_y_eventos() {
        let h = Historial::en_memoria().unwrap();
        h.guardar_lectura(1_000_000, &lectura(true, 24.5), true).unwrap();
        h.guardar_evento(1_000_060, "Apagado, \"prueba\"").unwrap();
        let mut salida = Vec::new();
        assert_eq!(h.exportar_csv(&mut salida).unwrap(), 2);
        let texto = String::from_utf8(salida).unwrap();
        let filas: Vec<&str> = texto.lines().collect();
        assert_eq!(filas.len(), 3);
        assert!(filas[0].starts_with("fecha_hora,encendido"));
        assert!(filas[1].contains(",24.5,30"));
        assert!(filas[2].ends_with("\"Apagado, \"\"prueba\"\"\""));
    }
}
