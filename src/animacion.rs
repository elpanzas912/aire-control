//! Transiciones del panel. Cada valor se calcula según la hora que le pasan, así que nada corre por
//! su cuenta: la ventana pide cuadros solo mientras alguna transición sigue en curso.
use crate::lienzo::Color;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curva {
    /// Entradas, salidas y respuestas a un toque: arranca rápido y frena al llegar.
    Salida,
    /// Algo que viaja de un lugar a otro de la pantalla.
    Movimiento,
    /// Cambios de color.
    Suave,
}

fn bezier(a: f32, b: f32, s: f32) -> f32 {
    let r = 1.0 - s;
    3.0 * r * r * s * a + 3.0 * r * s * s * b + s * s * s
}

impl Curva {
    fn puntos(self) -> (f32, f32, f32, f32) {
        match self {
            Self::Salida => (0.23, 1.0, 0.32, 1.0),
            Self::Movimiento => (0.77, 0.0, 0.175, 1.0),
            Self::Suave => (0.25, 0.1, 0.25, 1.0),
        }
    }

    pub fn aplicar(self, t: f32) -> f32 {
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        let (x1, y1, x2, y2) = self.puntos();
        // Por bisección: a diferencia de Newton, no se escapa en las curvas más cerradas
        let (mut bajo, mut alto) = (0.0f32, 1.0f32);
        for _ in 0..16 {
            let medio = (bajo + alto) / 2.0;
            if bezier(x1, x2, medio) < t {
                bajo = medio;
            } else {
                alto = medio;
            }
        }
        bezier(y1, y2, (bajo + alto) / 2.0)
    }
}

/// Un número que va de a poco hacia su destino. Si el destino cambia a mitad de camino, sigue desde
/// donde estaba, sin saltos.
#[derive(Clone, Copy, Debug)]
pub struct Valor {
    desde: f32,
    hasta: f32,
    inicio: f64,
    duracion: f32,
    curva: Curva,
}

impl Valor {
    pub const fn quieto(v: f32) -> Self {
        Self { desde: v, hasta: v, inicio: 0.0, duracion: 0.0, curva: Curva::Salida }
    }

    pub fn saltar(&mut self, v: f32) {
        *self = Self::quieto(v);
    }

    pub fn ir_a(&mut self, hasta: f32, ms: f32, curva: Curva, ahora: f64) {
        if self.hasta == hasta {
            return;
        }
        if ms <= 0.0 {
            self.saltar(hasta);
            return;
        }
        *self = Self { desde: self.en(ahora), hasta, inicio: ahora, duracion: ms, curva };
    }

    pub fn en(&self, ahora: f64) -> f32 {
        if self.duracion <= 0.0 {
            return self.hasta;
        }
        let t = ((ahora - self.inicio) as f32 / self.duracion).clamp(0.0, 1.0);
        self.desde + (self.hasta - self.desde) * self.curva.aplicar(t)
    }

    pub fn en_curso(&self, ahora: f64) -> bool {
        self.duracion > 0.0 && ahora - self.inicio < self.duracion as f64
    }
}

/// Lo mismo que `Valor`, para un color.
#[derive(Clone, Copy)]
pub struct Tono {
    desde: Color,
    hacia: Color,
    paso: Valor,
}

impl Tono {
    pub const fn quieto(color: Color) -> Self {
        Self { desde: color, hacia: color, paso: Valor::quieto(1.0) }
    }

    pub fn ir_a(&mut self, color: Color, ms: f32, ahora: f64) {
        if self.hacia == color {
            return;
        }
        self.desde = self.en(ahora);
        self.hacia = color;
        self.paso = Valor::quieto(0.0);
        self.paso.ir_a(1.0, ms, Curva::Suave, ahora);
    }

    pub fn en(&self, ahora: f64) -> Color {
        self.desde.mezcla(self.hacia, self.paso.en(ahora))
    }

    pub fn en_curso(&self, ahora: f64) -> bool {
        self.paso.en_curso(ahora)
    }
}

/// Cambia el alto de la ventana sin redimensionarla en cada cuadro: mientras dura la transición sube
/// o baja entera, y lo que sobra queda abajo, detrás de la barra de tareas.
#[derive(Clone, Copy, Debug)]
pub struct Cajon {
    alto: f32,
    destino: f32,
    desplazo: Valor,
}

impl Cajon {
    pub const fn fijo(alto: f32) -> Self {
        Self { alto, destino: alto, desplazo: Valor::quieto(0.0) }
    }

    pub fn alto(&self) -> f32 {
        self.alto
    }

    pub fn desplazo(&self, ahora: f64) -> f32 {
        self.desplazo.en(ahora)
    }

    pub fn en_curso(&self, ahora: f64) -> bool {
        self.desplazo.en_curso(ahora)
    }

    pub fn cambiar_alto(&mut self, nuevo: f32, ms: f32, ahora: f64) {
        self.destino = nuevo;
        if nuevo >= self.alto {
            // Crece ya, con lo nuevo escondido abajo, y sube: el borde de arriba arranca donde estaba
            let arriba = self.alto - self.desplazo.en(ahora);
            self.alto = nuevo;
            self.desplazo.saltar(nuevo - arriba);
            self.desplazo.ir_a(0.0, ms, Curva::Movimiento, ahora);
        } else {
            // Baja con el tamaño que tiene y se achica al final, cuando ya no se nota
            self.desplazo.ir_a(self.alto - nuevo, ms, Curva::Movimiento, ahora);
        }
    }

    /// Termina de achicar la ventana. Devuelve true si le cambió el tamaño.
    pub fn asentar(&mut self, ahora: f64) -> bool {
        if self.en_curso(ahora) || self.alto == self.destino {
            return false;
        }
        *self = Self::fijo(self.destino);
        true
    }
}

/// Cuánto tapa la píldora de una fila de opciones a la celda que está en `celda`, de 0 a 1.
pub fn cobertura(pildora: f32, celda: f32, ancho: f32) -> f32 {
    (1.0 - (pildora - celda).abs() / ancho).clamp(0.0, 1.0)
}

/// Ancho de cada tramo cuando hay `n` tramos, con decimales mientras uno aparece o se va.
/// Los tramos completos se angostan para que siempre llenen la línea.
pub fn tramos(n: f32, total: f32, hueco: f32) -> Vec<f32> {
    if n <= 0.0 {
        return Vec::new();
    }
    if n <= 1.0 {
        return vec![total * n];
    }
    let cantidad = n.ceil() as usize;
    let completo = (total - hueco * (cantidad - 1) as f32) / n;
    let mut anchos = vec![completo; cantidad];
    anchos[cantidad - 1] = completo * (n - (cantidad - 1) as f32);
    anchos
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const CURVAS: [Curva; 3] = [Curva::Salida, Curva::Movimiento, Curva::Suave];

    #[test]
    fn las_curvas_van_de_cero_a_uno_sin_retroceder() {
        for curva in CURVAS {
            assert_eq!(curva.aplicar(0.0), 0.0);
            assert_eq!(curva.aplicar(1.0), 1.0);
            let mut anterior = 0.0;
            for i in 1..=100 {
                let y = curva.aplicar(i as f32 / 100.0);
                assert!(y >= anterior - 0.001, "{curva:?} retrocede en {i}");
                anterior = y;
            }
        }
    }

    #[test]
    fn cada_curva_tiene_su_forma() {
        assert!(Curva::Salida.aplicar(0.2) > 0.6, "las entradas arrancan rápido");
        assert!(Curva::Movimiento.aplicar(0.15) < 0.05, "lo que viaja arranca suave");
        assert!(Curva::Movimiento.aplicar(0.85) > 0.95, "y llega suave");
    }

    #[test]
    fn un_valor_llega_a_destino_y_deja_de_correr() {
        let mut v = Valor::quieto(0.0);
        assert!(!v.en_curso(0.0));
        v.ir_a(10.0, 100.0, Curva::Salida, 50.0);
        assert_eq!(v.en(50.0), 0.0);
        assert!(v.en_curso(149.0));
        assert!(v.en(100.0) > 0.0 && v.en(100.0) < 10.0);
        assert_eq!(v.en(150.0), 10.0);
        assert_eq!(v.en(9000.0), 10.0);
        assert!(!v.en_curso(150.0));
    }

    #[test]
    fn cambiar_de_destino_a_mitad_de_camino_no_salta() {
        let mut v = Valor::quieto(0.0);
        v.ir_a(100.0, 200.0, Curva::Movimiento, 0.0);
        let antes = v.en(100.0);
        v.ir_a(0.0, 200.0, Curva::Movimiento, 100.0);
        assert_eq!(v.en(100.0), antes);
        assert_eq!(v.en(300.0), 0.0);
    }

    #[test]
    fn pedir_el_mismo_destino_no_reinicia_la_transicion() {
        let mut v = Valor::quieto(0.0);
        v.ir_a(10.0, 100.0, Curva::Salida, 0.0);
        let antes = v.en(50.0);
        v.ir_a(10.0, 100.0, Curva::Salida, 50.0);
        assert_eq!(v.en(50.0), antes);
    }

    #[test]
    fn sin_duracion_llega_enseguida() {
        let mut v = Valor::quieto(3.0);
        v.ir_a(7.0, 0.0, Curva::Salida, 10.0);
        assert_eq!(v.en(10.0), 7.0);
        assert!(!v.en_curso(10.0));
    }

    #[test]
    fn un_tono_pasa_de_un_color_al_otro_sin_saltos() {
        let (azul, naranja) = (Color::rgb(0x0000FF), Color::rgb(0xFF8000));
        let mut tono = Tono::quieto(azul);
        assert!(tono.en(0.0) == azul);
        tono.ir_a(naranja, 100.0, 0.0);
        assert!(tono.en(0.0) == azul);
        assert!(tono.en_curso(50.0));
        let medio = tono.en(50.0);
        assert!(medio != azul && medio != naranja);
        tono.ir_a(azul, 100.0, 50.0);
        assert!(tono.en(50.0) == medio);
        assert!(tono.en(150.0) == azul);
    }

    #[test]
    fn al_crecer_el_borde_de_arriba_arranca_donde_estaba() {
        let mut c = Cajon::fijo(500.0);
        c.cambiar_alto(600.0, 200.0, 0.0);
        assert_eq!(c.alto(), 600.0, "crece enseguida");
        assert_eq!(c.alto() - c.desplazo(0.0), 500.0);
        assert_eq!(c.alto() - c.desplazo(200.0), 600.0);
        assert!(!c.asentar(200.0));
    }

    #[test]
    fn al_achicarse_cambia_de_tamano_recien_al_final() {
        let mut c = Cajon::fijo(600.0);
        c.cambiar_alto(500.0, 200.0, 0.0);
        assert_eq!(c.alto(), 600.0);
        assert!(!c.asentar(100.0));
        assert_eq!(c.alto() - c.desplazo(200.0), 500.0);
        assert!(c.asentar(200.0));
        assert_eq!(c.alto(), 500.0);
        assert_eq!(c.desplazo(200.0), 0.0);
        assert!(!c.asentar(201.0), "solo una vez");
    }

    #[test]
    fn se_puede_cambiar_de_idea_a_mitad_de_camino() {
        let mut c = Cajon::fijo(600.0);
        c.cambiar_alto(500.0, 200.0, 0.0);
        let arriba = c.alto() - c.desplazo(100.0);
        c.cambiar_alto(600.0, 200.0, 100.0);
        assert_eq!(c.alto() - c.desplazo(100.0), arriba);
        assert_eq!(c.alto() - c.desplazo(300.0), 600.0);

        let mut c = Cajon::fijo(500.0);
        c.cambiar_alto(700.0, 200.0, 0.0);
        let arriba = c.alto() - c.desplazo(100.0);
        c.cambiar_alto(400.0, 200.0, 100.0);
        assert!((c.alto() - c.desplazo(100.0) - arriba).abs() < 0.01);
        assert!(c.asentar(300.0));
        assert_eq!(c.alto(), 400.0);
    }

    #[test]
    fn la_pildora_tapa_solo_lo_que_tiene_abajo() {
        assert_eq!(cobertura(100.0, 100.0, 50.0), 1.0);
        assert_eq!(cobertura(125.0, 100.0, 50.0), 0.5);
        assert_eq!(cobertura(75.0, 100.0, 50.0), 0.5);
        assert_eq!(cobertura(200.0, 100.0, 50.0), 0.0);
    }

    #[test]
    fn los_tramos_siempre_llenan_la_linea() {
        assert!(tramos(0.0, 100.0, 2.0).is_empty());
        assert_eq!(tramos(0.5, 100.0, 2.0), vec![50.0]);
        assert_eq!(tramos(1.0, 100.0, 2.0), vec![100.0]);
        for n in [1.25f32, 2.0, 2.5, 3.0, 3.75, 8.0] {
            let anchos = tramos(n, 100.0, 2.0);
            assert_eq!(anchos.len(), n.ceil() as usize);
            let ocupado: f32 = anchos.iter().sum::<f32>() + 2.0 * (anchos.len() - 1) as f32;
            assert!((ocupado - 100.0).abs() < 0.01, "con {n} tramos ocupa {ocupado}");
        }
    }
}
