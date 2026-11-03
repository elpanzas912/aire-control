//! Aire simulado e historial de ejemplo, solo para ver y probar el panel. Nunca toca el aire real.
use crate::historial::Historial;
use crate::nube::{Aire, ErrorNube, Transporte};
use crate::protocolo::*;

pub struct AireDemo {
    estado: Estado,
}

impl AireDemo {
    pub fn nuevo() -> Self {
        Self {
            estado: Estado {
                encendido: false,
                objetivo: 21.0,
                modo: MODO_FRIO,
                ventilador: VENT_AUTO,
                interior: Some(23.5),
                exterior: Some(25.0),
                pantalla: true,
                ..Default::default()
            },
        }
    }
}

impl Transporte for AireDemo {
    fn iniciar_sesion(&mut self) -> Result<(), ErrorNube> {
        Ok(())
    }

    fn enviar(&mut self, t: &[u8]) -> Result<Vec<u8>, ErrorNube> {
        if t[10] == 0x40 {
            self.estado.encendido = t[11] & 1 != 0;
            self.estado.objetivo = (t[12] & 0xF) as f32 + 16.0;
            self.estado.modo = t[12] >> 5;
            self.estado.ventilador = t[13];
        }
        let mut p = vec![0u8; 19];
        p[0] = 0xC0;
        p[1] = self.estado.encendido as u8;
        p[2] = ((self.estado.objetivo as u8).wrapping_sub(16) & 0xF) | (self.estado.modo << 5);
        p[3] = self.estado.ventilador;
        p[4] = 0x7F;
        p[5] = 0x7F;
        p[11] = 97;
        p[12] = 100;
        Ok(trama_de_respuesta(&p))
    }

    fn aires(&self) -> Vec<Aire> {
        vec![Aire { id: 1, nombre: "Cuarto".into() }, Aire { id: 2, nombre: "Living".into() }]
    }
}

pub fn sembrar(h: &Historial, ahora: i64) {
    let mut encendido = false;
    let mut interior = 25.0f32;
    let mut ts = ahora - 30 * 86_400;
    let mut paso = 0u32;
    while ts < ahora {
        let hora_dia = ((ts.rem_euclid(86_400)) as f32) / 3600.0;
        let exterior = 24.0 + 6.0 * ((hora_dia - 9.0) * std::f32::consts::PI / 12.0).sin() + ((paso % 13) as f32) * 0.05;
        let debe = !(7.0..=20.0).contains(&hora_dia);
        if debe != encendido {
            encendido = debe;
            let _ = h.guardar_evento(ts, if encendido { "Se prendió desde el temporizador" } else { "Se apagó desde el temporizador" });
        }
        interior += if encendido { (21.0 - interior) * 0.12 } else { (exterior - interior) * 0.03 };
        let e = Estado {
            encendido,
            objetivo: 21.0,
            modo: if paso % 40 < 34 { MODO_FRIO } else { MODO_VENTILAR },
            ventilador: VENT_AUTO,
            interior: Some((interior * 2.0).round() / 2.0),
            exterior: Some((exterior * 2.0).round() / 2.0),
            pantalla: true,
            crudo: vec![0xC0],
            ..Default::default()
        };
        // Falta un tramo a propósito, para ver cómo queda un hueco de datos
        if !(ahora - 16 * 3600..ahora - 13 * 3600).contains(&ts) {
            let _ = h.guardar_lectura(ts, &e, false);
        }
        ts += 300;
        paso += 1;
    }
}
