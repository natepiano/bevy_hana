/// Fixed and scientific text for a value rounded to three significant digits.
pub struct ThreeSignificantDigits {
    /// The rounded value in fixed notation.
    pub fixed:      String,
    /// The rounded value in scientific notation.
    pub scientific: String,
}

impl From<f64> for ThreeSignificantDigits {
    fn from(value: f64) -> Self {
        let scientific = format!("{value:.2e}");
        let fallback = || Self {
            fixed:      scientific.clone(),
            scientific: scientific.clone(),
        };
        if !value.is_finite() {
            let text = value.to_string();
            return Self {
                fixed:      text.clone(),
                scientific: text,
            };
        }
        if value == 0.0 {
            return Self {
                fixed: String::from("0.00"),
                scientific,
            };
        }
        let Some((_, exponent)) = scientific.rsplit_once('e') else {
            return fallback();
        };
        let Ok(exponent) = exponent.parse::<i32>() else {
            return fallback();
        };
        let Ok(rounded) = scientific.parse::<f64>() else {
            return fallback();
        };
        let decimals = usize::try_from(2_i32.saturating_sub(exponent)).unwrap_or_default();
        Self {
            fixed: format!("{rounded:.decimals$}"),
            scientific,
        }
    }
}
