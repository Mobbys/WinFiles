//! Unità di misura mostrate all'utente e formattazione dei numeri all'italiana (virgola decimale,
//! punto per le migliaia). I calcoli restano in millimetri: qui si converte solo per mostrare.

use serde::{Deserialize, Serialize};

use crate::Size;

/// Unità in cui mostrare le lunghezze.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Unit {
    #[default]
    Mm,
    Cm,
}

impl Unit {
    pub const ALL: [Unit; 2] = [Unit::Mm, Unit::Cm];

    pub const fn symbol(self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::Cm => "cm",
        }
    }

    /// Millimetri in un'unità.
    pub const fn mm(self) -> f64 {
        match self {
            Unit::Mm => 1.0,
            Unit::Cm => 10.0,
        }
    }

    /// Decimali mostrati: un decimo di millimetro basta in entrambe le unità.
    pub const fn decimals(self) -> usize {
        match self {
            Unit::Mm => 1,
            Unit::Cm => 2,
        }
    }
}

/// Area in metri quadrati di un rettangolo misurato in millimetri.
pub fn area_m2(size: Size) -> f64 {
    let a = size.w * size.h / 1_000_000.0;
    if a.is_finite() && a > 0.0 { a } else { 0.0 }
}

/// Un numero con al massimo `decimals` decimali, virgola decimale e punto per le migliaia.
/// Con `trim` toglie gli zeri finali (`21,50` → `21,5`, `210,0` → `210`).
pub fn format_number(value: f64, decimals: usize, trim: bool) -> String {
    if !value.is_finite() {
        return "–".to_string();
    }
    let text = format!("{:.*}", decimals, value.abs());
    let (int, frac) = text.split_once('.').unwrap_or((text.as_str(), ""));
    let frac = if trim { frac.trim_end_matches('0') } else { frac };
    let mut out = String::with_capacity(text.len() + 4);
    // Lo zero negativo ("-0,0") non ha senso per l'utente.
    if value < 0.0 && (int.bytes().any(|b| b != b'0') || frac.bytes().any(|b| b != b'0')) {
        out.push('-');
    }
    let digits = int.len();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (digits - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    if !frac.is_empty() {
        out.push(',');
        out.push_str(frac);
    }
    out
}

/// Una lunghezza in millimetri nell'unità scelta, con il simbolo: `29,7 cm`.
pub fn format_length(mm: f64, unit: Unit) -> String {
    format!(
        "{} {}",
        format_number(mm / unit.mm(), unit.decimals(), true),
        unit.symbol()
    )
}

/// Base × altezza nell'unità scelta: `21 × 29,7 cm`.
pub fn format_size(size: Size, unit: Unit) -> String {
    format!(
        "{} × {} {}",
        format_number(size.w / unit.mm(), unit.decimals(), true),
        format_number(size.h / unit.mm(), unit.decimals(), true),
        unit.symbol()
    )
}

/// Un'area in metri quadrati, sempre con tre decimali così le colonne si leggono allineate.
pub fn format_area(m2: f64) -> String {
    format!("{} m²", format_number(m2, 3, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_use_italian_separators() {
        assert_eq!(format_number(1234567.891, 2, false), "1.234.567,89");
        assert_eq!(format_number(210.0, 1, true), "210");
        assert_eq!(format_number(21.5, 2, true), "21,5");
        assert_eq!(format_number(0.0626, 3, false), "0,063");
        assert_eq!(format_number(-0.0001, 1, true), "0");
        assert_eq!(format_number(-2.5, 1, true), "-2,5");
        assert_eq!(format_number(f64::NAN, 1, true), "–");
        assert_eq!(format_number(999.0, 0, true), "999");
        assert_eq!(format_number(1000.0, 0, true), "1.000");
    }

    #[test]
    fn a4_in_both_units() {
        let a4 = Size::new(210.0, 297.0);
        assert_eq!(format_size(a4, Unit::Mm), "210 × 297 mm");
        assert_eq!(format_size(a4, Unit::Cm), "21 × 29,7 cm");
        assert_eq!(format_length(215.9, Unit::Cm), "21,59 cm");
        assert_eq!(format_area(area_m2(a4)), "0,062 m²");
    }

    #[test]
    fn area_of_a0_is_one_square_metre() {
        let a = area_m2(Size::new(841.0, 1189.0));
        assert!((a - 1.0).abs() < 0.001);
        assert_eq!(area_m2(Size::new(-1.0, 5.0)), 0.0);
        assert_eq!(area_m2(Size::new(f64::INFINITY, 5.0)), 0.0);
    }
}
