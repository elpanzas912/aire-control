//! Ícono en la bandeja del sistema: muestra la temperatura del cuarto y el color del modo.
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const MENSAJE_BANDEJA: u32 = WM_APP + 2;
const ID_ICONO: u32 = 1;

fn copiar(destino: &mut [u16], texto: &str) {
    let tope = destino.len() - 1;
    let mut n = 0;
    for unidad in texto.encode_utf16().take(tope) {
        destino[n] = unidad;
        n += 1;
    }
    destino[n] = 0;
}

fn base(ventana: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: ventana,
        uID: ID_ICONO,
        ..Default::default()
    }
}

/// Dibuja una pastilla redondeada con el número. Se hace a mano para tener bordes suaves con alfa.
pub fn crear_icono(texto: &str, fondo: u32) -> Option<HICON> {
    unsafe {
        let dpi = GetDpiForSystem();
        let lado = GetSystemMetricsForDpi(SM_CXSMICON, dpi).max(16);
        let dc = CreateCompatibleDC(None);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: lado,
                biHeight: -lado,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixeles: *mut core::ffi::c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut pixeles, None, 0).ok()?;
        let anterior = SelectObject(dc, dib.into());

        // Con letras blancas sobre negro, el brillo de cada píxel dice cuánto lo cubre el texto
        let alto_letra = (lado as f32 * 0.82) as i32;
        let fuente = CreateFontW(
            -alto_letra, 0, 0, 0, FW_SEMIBOLD.0 as i32, 0, 0, 0,
            DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY,
            0, w!("Bahnschrift"),
        );
        let fuente_previa = SelectObject(dc, fuente.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(0x00FF_FFFF));
        let mut letras: Vec<u16> = texto.encode_utf16().collect();
        let mut zona = RECT { left: 0, top: 0, right: lado, bottom: lado };
        DrawTextW(dc, &mut letras, &mut zona, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, fuente_previa);
        let _ = DeleteObject(fuente.into());

        let cantidad = (lado * lado) as usize;
        let datos = std::slice::from_raw_parts_mut(pixeles as *mut u32, cantidad);
        let radio = lado as f32 * 0.24;
        let (r, g, b) = (((fondo >> 16) & 0xFF) as f32, ((fondo >> 8) & 0xFF) as f32, (fondo & 0xFF) as f32);
        let (lr, lg, lb) = (13.0, 17.0, 23.0);
        for y in 0..lado {
            for x in 0..lado {
                let i = (y * lado + x) as usize;
                let letra = (datos[i] & 0xFF) as f32 / 255.0;
                // Distancia al borde de un rectángulo redondeado
                let centro = lado as f32 / 2.0;
                let dx = (x as f32 + 0.5 - centro).abs() - (centro - radio);
                let dy = (y as f32 + 0.5 - centro).abs() - (centro - radio);
                let afuera = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - radio;
                let cobertura = (0.5 - afuera).clamp(0.0, 1.0);
                let mezcla = |c: f32, l: f32| (c * (1.0 - letra) + l * letra) * cobertura;
                let (pr, pg, pb) = (mezcla(r, lr), mezcla(g, lg), mezcla(b, lb));
                datos[i] = ((cobertura * 255.0) as u32) << 24
                    | (pr as u32) << 16
                    | (pg as u32) << 8
                    | pb as u32;
            }
        }
        SelectObject(dc, anterior);
        let _ = DeleteDC(dc);

        let mascara = CreateBitmap(lado, lado, 1, 1, None);
        let datos_icono = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mascara,
            hbmColor: dib,
        };
        let icono = CreateIconIndirect(&datos_icono).ok();
        let _ = DeleteObject(mascara.into());
        let _ = DeleteObject(dib.into());
        icono
    }
}

pub fn agregar(ventana: HWND, icono: HICON, tooltip: &str) {
    let mut datos = base(ventana);
    datos.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    datos.uCallbackMessage = MENSAJE_BANDEJA;
    datos.hIcon = icono;
    copiar(&mut datos.szTip, tooltip);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &datos);
    }
}

pub fn actualizar(ventana: HWND, icono: HICON, tooltip: &str) {
    let mut datos = base(ventana);
    datos.uFlags = NIF_ICON | NIF_TIP;
    datos.hIcon = icono;
    copiar(&mut datos.szTip, tooltip);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &datos);
    }
}

pub fn avisar(ventana: HWND, titulo: &str, texto: &str) {
    let mut datos = base(ventana);
    datos.uFlags = NIF_INFO;
    datos.dwInfoFlags = NIIF_NONE;
    copiar(&mut datos.szInfoTitle, titulo);
    copiar(&mut datos.szInfo, texto);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &datos);
    }
}

pub fn quitar(ventana: HWND) {
    let datos = base(ventana);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &datos);
    }
}

pub fn rectangulo(ventana: HWND) -> Option<RECT> {
    let identificador = NOTIFYICONIDENTIFIER {
        cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: ventana,
        uID: ID_ICONO,
        ..Default::default()
    };
    unsafe { Shell_NotifyIconGetRect(&identificador).ok() }
}

/// Muestra el menú y devuelve el comando elegido (0 si se cerró sin elegir).
pub fn mostrar_menu(ventana: HWND, opciones: &[(u32, &str, bool)]) -> u32 {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return 0 };
        for (id, texto, marcada) in opciones {
            if *id == 0 {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            } else {
                let ancho: Vec<u16> = texto.encode_utf16().chain(Some(0)).collect();
                let marca = if *marcada { MF_CHECKED } else { MF_UNCHECKED };
                let _ = AppendMenuW(menu, MF_STRING | marca, *id as usize, PCWSTR(ancho.as_ptr()));
            }
        }
        let mut punto = Default::default();
        let _ = GetCursorPos(&mut punto);
        // Sin esto el menú no se cierra al hacer clic afuera
        let _ = SetForegroundWindow(ventana);
        let elegido = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            punto.x,
            punto.y,
            None,
            ventana,
            None,
        );
        let _ = DestroyMenu(menu);
        elegido.0 as u32
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_icono_es_una_pastilla_con_alfa() {
        let icono = crear_icono("23", 0x79C6F5).expect("icono");
        unsafe {
            let mut datos = ICONINFO::default();
            GetIconInfo(icono, &mut datos).unwrap();
            let mut bitmap = BITMAP::default();
            GetObjectW(datos.hbmColor.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bitmap as *mut _ as *mut _));
            let lado = bitmap.bmWidth;
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: lado,
                    biHeight: -lado,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixeles = vec![0u32; (lado * lado) as usize];
            let dc = CreateCompatibleDC(None);
            GetDIBits(dc, datos.hbmColor, 0, lado as u32, Some(pixeles.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
            let _ = DeleteDC(dc);
            let _ = DestroyIcon(icono);
            let alfa = |x: i32, y: i32| pixeles[(y * lado + x) as usize] >> 24;
            assert_eq!(alfa(0, 0), 0, "la esquina es transparente");
            assert_eq!(alfa(lado / 2, 1), 255, "el borde recto es opaco");
            assert!(pixeles.iter().any(|p| (p & 0xFF) < 60 && p >> 24 == 255), "hay letra oscura");
            if let Some(ruta) = std::env::var_os("AIRE_VOLCAR_ICONO") {
                let mut salida = lado.to_le_bytes().to_vec();
                for p in &pixeles {
                    salida.extend_from_slice(&p.to_le_bytes());
                }
                std::fs::write(ruta, salida).unwrap();
            }
        }
    }
}
