//! Dibujo con Direct2D y DirectWrite. Se crea al abrir el panel y se libera al cerrarlo,
//! así la app en la bandeja no gasta memoria en gráficos.
use std::collections::HashMap;

use windows::core::{w, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows_numerics::{Matrix3x2, Vector2};

pub const FUENTE: PCWSTR = w!("Bahnschrift");
pub const ICONOS: PCWSTR = w!("Segoe Fluent Icons");
/// Windows 10 no trae la anterior, pero sí esta, con los mismos códigos.
const ICONOS_WINDOWS_10: PCWSTR = w!("Segoe MDL2 Assets");

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Alinear {
    Izquierda,
    Centro,
    Derecha,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Peso {
    Liviano,
    Normal,
    Medio,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Caja {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Caja {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contiene(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + self.h }
    }

    pub fn adentro(&self, margen: f32) -> Self {
        Self::new(self.x + margen, self.y + margen, self.w - margen * 2.0, self.h - margen * 2.0)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct Color(pub u32, pub f32);

impl Color {
    pub const fn rgb(hex: u32) -> Self {
        Self(hex, 1.0)
    }

    pub const fn con_alfa(self, alfa: f32) -> Self {
        Self(self.0, alfa)
    }

    pub fn mezcla(self, otro: Self, t: f32) -> Self {
        let canal = |corrimiento: u32| {
            let a = ((self.0 >> corrimiento) & 0xFF) as f32;
            let b = ((otro.0 >> corrimiento) & 0xFF) as f32;
            ((a + (b - a) * t).round() as u32) << corrimiento
        };
        Self(canal(16) | canal(8) | canal(0), self.1 + (otro.1 - self.1) * t)
    }

    fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F {
            r: ((self.0 >> 16) & 0xFF) as f32 / 255.0,
            g: ((self.0 >> 8) & 0xFF) as f32 / 255.0,
            b: (self.0 & 0xFF) as f32 / 255.0,
            a: self.1,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ClaveFormato {
    tamano: u32,
    peso: Peso,
    alinear: Alinear,
    iconos: bool,
    recortar: bool,
    ajustar: bool,
}

/// Se conservan entre aperturas: recrearlas no libera más y fragmenta la memoria.
#[derive(Clone)]
pub struct Fabricas {
    dibujo: ID2D1Factory,
    escritura: IDWriteFactory,
}

impl Fabricas {
    pub fn nuevas() -> Result<Self> {
        unsafe {
            Ok(Self {
                dibujo: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                escritura: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
            })
        }
    }
}

pub struct Lienzo {
    fabrica: ID2D1Factory,
    escritura: IDWriteFactory,
    destino: ID2D1HwndRenderTarget,
    pincel: ID2D1SolidColorBrush,
    trazo_redondo: ID2D1StrokeStyle,
    formatos: HashMap<ClaveFormato, IDWriteTextFormat>,
    opacidad: f32,
}

fn punto(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

fn ancho(texto: &str) -> Vec<u16> {
    texto.encode_utf16().collect()
}

impl Lienzo {
    pub fn nuevo(ventana: HWND, fabricas: &Fabricas) -> Result<Self> {
        unsafe {
            let Fabricas { dibujo: fabrica, escritura } = fabricas.clone();

            let mut cliente = Default::default();
            GetClientRect(ventana, &mut cliente)?;
            let dpi = GetDpiForWindow(ventana) as f32;
            // Por software: el panel es chico y así no se carga el controlador de la placa de video
            let propiedades = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: dpi,
                dpiY: dpi,
                ..Default::default()
            };
            let ventana_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: ventana,
                pixelSize: D2D_SIZE_U {
                    width: (cliente.right - cliente.left).max(1) as u32,
                    height: (cliente.bottom - cliente.top).max(1) as u32,
                },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            let destino = fabrica.CreateHwndRenderTarget(&propiedades, &ventana_props)?;
            destino.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let pincel = destino.CreateSolidColorBrush(&Color::rgb(0xFFFFFF).d2d(), None)?;
            let trazo_redondo = fabrica.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 1.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                },
                None,
            )?;
            Ok(Self { fabrica, escritura, destino, pincel, trazo_redondo, formatos: HashMap::new(), opacidad: 1.0 })
        }
    }

    pub fn ajustar(&self, ancho_px: u32, alto_px: u32, dpi: f32) -> Result<()> {
        unsafe {
            self.destino.SetDpi(dpi, dpi);
            self.destino.Resize(&D2D_SIZE_U { width: ancho_px.max(1), height: alto_px.max(1) })
        }
    }

    pub fn empezar(&self) {
        unsafe {
            self.destino.BeginDraw();
            self.destino.Clear(Some(&Color(0, 0.0).d2d()));
        }
    }

    pub fn terminar(&self) -> Result<()> {
        unsafe { self.destino.EndDraw(None, None) }
    }

    /// Afecta a todo lo que se dibuje después, para fundir una vista entera.
    pub fn fijar_opacidad(&mut self, opacidad: f32) {
        self.opacidad = opacidad;
    }

    /// Encoge o agranda lo que se dibuje después, desde el centro de `c`.
    pub fn escalar(&self, c: Caja, factor: f32) {
        let (x, y) = (c.x + c.w / 2.0, c.y + c.h / 2.0);
        let matriz = Matrix3x2 {
            M11: factor,
            M12: 0.0,
            M21: 0.0,
            M22: factor,
            M31: x * (1.0 - factor),
            M32: y * (1.0 - factor),
        };
        unsafe { self.destino.SetTransform(&matriz) };
    }

    /// Gira lo que se dibuje después alrededor del centro de `c`.
    pub fn girar(&self, c: Caja, radianes: f32) {
        let (x, y) = (c.x + c.w / 2.0, c.y + c.h / 2.0);
        let (seno, coseno) = radianes.sin_cos();
        let matriz = Matrix3x2 {
            M11: coseno,
            M12: seno,
            M21: -seno,
            M22: coseno,
            M31: x - x * coseno + y * seno,
            M32: y - x * seno - y * coseno,
        };
        unsafe { self.destino.SetTransform(&matriz) };
    }

    pub fn sin_transformar(&self) {
        let matriz = Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 };
        unsafe { self.destino.SetTransform(&matriz) };
    }

    fn con_color(&self, color: Color) -> &ID2D1SolidColorBrush {
        unsafe { self.pincel.SetColor(&color.con_alfa(color.1 * self.opacidad).d2d()) };
        &self.pincel
    }

    pub fn caja(&self, c: Caja, radio: f32, color: Color) {
        unsafe {
            let redondeada = D2D1_ROUNDED_RECT { rect: c.d2d(), radiusX: radio, radiusY: radio };
            self.destino.FillRoundedRectangle(&redondeada, self.con_color(color));
        }
    }

    pub fn borde(&self, c: Caja, radio: f32, grosor: f32, color: Color) {
        unsafe {
            let redondeada = D2D1_ROUNDED_RECT { rect: c.d2d(), radiusX: radio, radiusY: radio };
            self.destino.DrawRoundedRectangle(&redondeada, self.con_color(color), grosor, None);
        }
    }

    pub fn circulo(&self, x: f32, y: f32, radio: f32, color: Color) {
        unsafe {
            let elipse = D2D1_ELLIPSE { point: punto(x, y), radiusX: radio, radiusY: radio };
            self.destino.FillEllipse(&elipse, self.con_color(color));
        }
    }

    pub fn anillo(&self, x: f32, y: f32, radio: f32, grosor: f32, color: Color) {
        unsafe {
            let elipse = D2D1_ELLIPSE { point: punto(x, y), radiusX: radio, radiusY: radio };
            self.destino.DrawEllipse(&elipse, self.con_color(color), grosor, None);
        }
    }

    pub fn linea(&self, x1: f32, y1: f32, x2: f32, y2: f32, grosor: f32, color: Color) {
        unsafe {
            self.destino.DrawLine(
                punto(x1, y1),
                punto(x2, y2),
                self.con_color(color),
                grosor,
                &self.trazo_redondo,
            );
        }
    }

    /// Línea continua con uniones redondas, para las series del gráfico.
    pub fn trazo(&self, puntos: &[(f32, f32)], grosor: f32, color: Color) -> Result<()> {
        if puntos.len() < 2 {
            if let Some(&(x, y)) = puntos.first() {
                self.circulo(x, y, grosor, color);
            }
            return Ok(());
        }
        unsafe {
            let camino = self.fabrica.CreatePathGeometry()?;
            let hoja = camino.Open()?;
            hoja.BeginFigure(punto(puntos[0].0, puntos[0].1), D2D1_FIGURE_BEGIN_HOLLOW);
            let resto: Vec<Vector2> = puntos[1..].iter().map(|&(x, y)| punto(x, y)).collect();
            hoja.AddLines(&resto);
            hoja.EndFigure(D2D1_FIGURE_END_OPEN);
            hoja.Close()?;
            self.destino.DrawGeometry(&camino, self.con_color(color), grosor, &self.trazo_redondo);
        }
        Ok(())
    }

    fn familia_iconos(&self) -> PCWSTR {
        unsafe {
            let mut coleccion = None;
            let (mut indice, mut existe) = (0u32, BOOL(0));
            let hay = self.escritura.GetSystemFontCollection(&mut coleccion, false).is_ok()
                && coleccion.is_some_and(|c: IDWriteFontCollection| {
                    c.FindFamilyName(ICONOS, &mut indice, &mut existe).is_ok() && existe.as_bool()
                });
            if hay {
                ICONOS
            } else {
                ICONOS_WINDOWS_10
            }
        }
    }

    fn formato(&mut self, clave: ClaveFormato) -> Result<IDWriteTextFormat> {
        if let Some(existente) = self.formatos.get(&clave) {
            return Ok(existente.clone());
        }
        unsafe {
            let peso = match clave.peso {
                Peso::Liviano => DWRITE_FONT_WEIGHT_LIGHT,
                Peso::Normal => DWRITE_FONT_WEIGHT_NORMAL,
                Peso::Medio => DWRITE_FONT_WEIGHT_SEMI_BOLD,
            };
            let familia = if clave.iconos { self.familia_iconos() } else { FUENTE };
            let formato = self.escritura.CreateTextFormat(
                familia,
                None,
                peso,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                clave.tamano as f32 / 10.0,
                w!("es-AR"),
            )?;
            formato.SetTextAlignment(match clave.alinear {
                Alinear::Izquierda => DWRITE_TEXT_ALIGNMENT_LEADING,
                Alinear::Centro => DWRITE_TEXT_ALIGNMENT_CENTER,
                Alinear::Derecha => DWRITE_TEXT_ALIGNMENT_TRAILING,
            })?;
            formato.SetParagraphAlignment(if clave.ajustar {
                DWRITE_PARAGRAPH_ALIGNMENT_NEAR
            } else {
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER
            })?;
            formato.SetWordWrapping(if clave.ajustar { DWRITE_WORD_WRAPPING_WRAP } else { DWRITE_WORD_WRAPPING_NO_WRAP })?;
            if clave.recortar {
                let signo = self.escritura.CreateEllipsisTrimmingSign(&formato)?;
                let recorte = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
                formato.SetTrimming(&recorte, &signo)?;
            }
            self.formatos.insert(clave, formato.clone());
            Ok(formato)
        }
    }

    fn dibujar_texto(&mut self, texto: &str, c: Caja, clave: ClaveFormato, color: Color) {
        let Ok(formato) = self.formato(clave) else { return };
        let unidades = ancho(texto);
        unsafe {
            self.destino.DrawText(
                &unidades,
                &formato,
                &c.d2d(),
                self.con_color(color),
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    pub fn texto(&mut self, texto: &str, c: Caja, tamano: f32, peso: Peso, alinear: Alinear, color: Color) {
        let clave = ClaveFormato { tamano: (tamano * 10.0) as u32, peso, alinear, iconos: false, recortar: false, ajustar: false };
        self.dibujar_texto(texto, c, clave, color);
    }

    /// Una línea que termina en "…" si no entra.
    pub fn texto_recortado(&mut self, texto: &str, c: Caja, tamano: f32, color: Color) {
        let clave = ClaveFormato { tamano: (tamano * 10.0) as u32, peso: Peso::Normal, alinear: Alinear::Izquierda, iconos: false, recortar: true, ajustar: false };
        self.dibujar_texto(texto, c, clave, color);
    }

    pub fn parrafo(&mut self, texto: &str, c: Caja, tamano: f32, color: Color) {
        let clave = ClaveFormato { tamano: (tamano * 10.0) as u32, peso: Peso::Normal, alinear: Alinear::Izquierda, iconos: false, recortar: false, ajustar: true };
        self.dibujar_texto(texto, c, clave, color);
    }

    pub fn icono(&mut self, codigo: u32, c: Caja, tamano: f32, color: Color) {
        let texto = char::from_u32(codigo).map(String::from).unwrap_or_default();
        let clave = ClaveFormato { tamano: (tamano * 10.0) as u32, peso: Peso::Normal, alinear: Alinear::Centro, iconos: true, recortar: false, ajustar: false };
        self.dibujar_texto(&texto, c, clave, color);
    }

    pub fn medir(&mut self, texto: &str, tamano: f32, peso: Peso) -> f32 {
        let clave = ClaveFormato { tamano: (tamano * 10.0) as u32, peso, alinear: Alinear::Izquierda, iconos: false, recortar: false, ajustar: false };
        let Ok(formato) = self.formato(clave) else { return 0.0 };
        let unidades = ancho(texto);
        unsafe {
            let Ok(diseno) = self.escritura.CreateTextLayout(&unidades, &formato, 2000.0, 200.0) else { return 0.0 };
            let mut metricas = DWRITE_TEXT_METRICS::default();
            if diseno.GetMetrics(&mut metricas).is_ok() {
                metricas.widthIncludingTrailingWhitespace
            } else {
                0.0
            }
        }
    }
}
