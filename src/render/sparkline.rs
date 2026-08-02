/// Minimal server-rendered inline SVG sparkline — no chart library, no JS.
/// `stroke="currentColor"` so it inherits the surrounding text color and
/// follows light/dark mode for free.
pub fn sparkline_svg(values: &[i64], width: u32, height: u32) -> String {
    if values.is_empty() {
        return format!(
            "<svg width=\"{width}\" height=\"{height}\" xmlns=\"http://www.w3.org/2000/svg\"></svg>"
        );
    }

    let max = values.iter().copied().max().unwrap_or(0).max(1);
    let n = values.len();
    let step = if n > 1 {
        f64::from(width) / (n - 1) as f64
    } else {
        0.0
    };

    let points: Vec<String> = values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = i as f64 * step;
            let y = f64::from(height) - (v as f64 / max as f64) * f64::from(height);
            format!("{x:.1},{y:.1}")
        })
        .collect();

    format!(
        "<svg width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\" \
         xmlns=\"http://www.w3.org/2000/svg\" class=\"sparkline\" preserveAspectRatio=\"none\">\
         <polyline fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" points=\"{}\"/>\
         </svg>",
        points.join(" "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_values_still_produce_valid_svg() {
        let svg = sparkline_svg(&[], 300, 60);
        assert!(svg.starts_with("<svg"));
    }

    #[test]
    fn flat_zero_series_does_not_divide_by_zero() {
        let svg = sparkline_svg(&[0, 0, 0], 300, 60);
        assert!(svg.contains("<polyline"));
        assert!(!svg.contains("NaN"));
    }
}
