//! Pesos argentinos. El precio del kWh se guarda en centavos para no arrastrar errores de coma flotante.

/// "60.720": separa los miles con punto, como se escribe en Argentina.
fn miles(n: i64) -> String {
    let digitos = n.max(0).to_string();
    let mut salida = String::with_capacity(digitos.len() + digitos.len() / 3);
    for (i, c) in digitos.chars().enumerate() {
        if i > 0 && (digitos.len() - i) % 3 == 0 {
            salida.push('.');
        }
        salida.push(c);
    }
    salida
}

/// Un monto redondeado al peso: "$ 60.720".
pub fn pesos(valor: f64) -> String {
    format!("$ {}", miles(valor.max(0.0).round() as i64))
}

/// El precio del kWh como se escribe en una factura: "197,95".
pub fn precio_escrito(centavos: i64) -> String {
    let centavos = centavos.max(0);
    format!("{},{:02}", miles(centavos / 100), centavos % 100)
}

/// Lee el precio que escribió la persona, en centavos: "197,95", "$ 1.234,5", "163,597" o "197.95".
/// Con coma, los puntos separan miles. Sin coma, un solo punto es decimal, porque así escribe la
/// calculadora; varios puntos solo pueden separar miles. El tercer decimal redondea.
pub fn leer_precio(texto: &str) -> Option<i64> {
    let limpio: String = texto.trim().trim_start_matches('$').chars().filter(|c| !c.is_whitespace()).collect();
    if limpio.is_empty() || !limpio.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',') {
        return None;
    }
    let (entera, decimales) = match (limpio.rfind(','), limpio.matches('.').count()) {
        (Some(coma), _) => (limpio[..coma].replace('.', ""), limpio[coma + 1..].to_string()),
        (None, 1) => {
            let punto = limpio.find('.')?;
            (limpio[..punto].to_string(), limpio[punto + 1..].to_string())
        }
        (None, _) => (limpio.replace('.', ""), String::new()),
    };
    if entera.contains(',') || decimales.contains(['.', ',']) || (entera.is_empty() && decimales.is_empty()) {
        return None;
    }
    let pesos: i64 = if entera.is_empty() { 0 } else { entera.parse().ok()? };
    let cifra = |i: usize| decimales.as_bytes().get(i).map_or(0, |b| (b - b'0') as i64);
    let redondeo = if cifra(2) >= 5 { 1 } else { 0 };
    pesos.checked_mul(100)?.checked_add(cifra(0) * 10 + cifra(1) + redondeo)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn montos_con_miles() {
        assert_eq!(pesos(0.4), "$ 0");
        assert_eq!(pesos(735.5), "$ 736");
        assert_eq!(pesos(60_720.2), "$ 60.720");
        assert_eq!(pesos(1_234_567.0), "$ 1.234.567");
        assert_eq!(pesos(-3.0), "$ 0");
    }

    #[test]
    fn precio_con_centavos() {
        assert_eq!(precio_escrito(19_795), "197,95");
        assert_eq!(precio_escrito(5), "0,05");
        assert_eq!(precio_escrito(123_456), "1.234,56");
    }

    #[test]
    fn lee_como_se_escribe_en_argentina() {
        assert_eq!(leer_precio("197,95"), Some(19_795));
        assert_eq!(leer_precio(" $ 197,95 "), Some(19_795));
        assert_eq!(leer_precio("163,597"), Some(16_360));
        assert_eq!(leer_precio("1.234,5"), Some(123_450));
        assert_eq!(leer_precio("200"), Some(20_000));
        assert_eq!(leer_precio(",5"), Some(50));
    }

    #[test]
    fn lee_el_punto_de_la_calculadora() {
        assert_eq!(leer_precio("197.95"), Some(19_795));
        assert_eq!(leer_precio("163.597"), Some(16_360));
        assert_eq!(leer_precio("1.234.567"), Some(123_456_700));
    }

    #[test]
    fn rechaza_lo_que_no_es_un_precio() {
        for texto in ["", "$", ",", "abc", "12a", "1,2,3", "1,2.3", "197,95 $"] {
            assert_eq!(leer_precio(texto), None, "{texto:?}");
        }
    }
}
