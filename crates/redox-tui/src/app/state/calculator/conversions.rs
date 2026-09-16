use super::{evaluate_number, format_number};

pub(super) fn evaluate(source: &str, target: &str) -> Result<String, &'static str> {
    let target = unit_name(target.strip_prefix('_').unwrap_or(target));
    let (value, source) = if let Some(value) = source
        .strip_prefix("rgb(")
        .and_then(|value| value.strip_suffix(')'))
    {
        (value, "rgb")
    } else if let Some(parts) = split_source(source) {
        parts
    } else if source.starts_with('#') {
        (source, "hex")
    } else {
        return Err("use value source to target for conversions");
    };
    let value = value.trim();
    let source = unit_name(source.trim());
    match (source, target) {
        ("rgb", "hex" | "hexadecimal") | ("hex" | "hexadecimal", "rgb") => {
            convert_color(value, source)
        }
        _ if base(source).is_some() || base(target).is_some() => {
            convert_base(value, source, target)
        }
        _ => convert_unit(evaluate_number(value)?, source, target),
    }
}

fn split_source(source: &str) -> Option<(&str, &str)> {
    if let Some(parts) = source.split_once('_') {
        return Some(parts);
    }
    // Try the complete unit name so expressions and units may both contain spaces.
    for (position, _) in source.match_indices(char::is_whitespace) {
        let name = source[position..].trim();
        let canonical = unit_name(name);
        if unit(canonical).is_some() || matches!(canonical, "C" | "F" | "K") {
            return Some((&source[..position], name));
        }
    }
    source.rsplit_once(char::is_whitespace)
}

fn unit_name(name: &str) -> &str {
    // Only full names and word abbreviations ignore case; symbols retain theirs.
    match name.to_ascii_lowercase().as_str() {
        "millimetre" | "millimetres" | "millimeter" | "millimeters" => "mm",
        "centimetre" | "centimetres" | "centimeter" | "centimeters" => "cm",
        "metre" | "metres" | "meter" | "meters" => "m",
        "kilometre" | "kilometres" | "kilometer" | "kilometers" => "km",
        "inch" | "inches" => "in",
        "foot" | "feet" => "ft",
        "yard" | "yards" => "yd",
        "mile" | "miles" => "mi",
        "nautical mile" | "nautical miles" => "nmi",
        "milligram" | "milligrams" => "mg",
        "gram" | "grams" => "g",
        "kilogram" | "kilograms" => "kg",
        "tonne" | "tonnes" | "metric tonne" | "metric tonnes" | "metric ton" | "metric tons" => "t",
        "ounce" | "ounces" => "oz",
        "pound" | "pounds" => "lb",
        "stone" | "stones" => "st",
        "square millimetre" | "square millimetres" | "square millimeter" | "square millimeters" => {
            "mm2"
        }
        "square centimetre" | "square centimetres" | "square centimeter" | "square centimeters" => {
            "cm2"
        }
        "square metre" | "square metres" | "square meter" | "square meters" => "m2",
        "square kilometre" | "square kilometres" | "square kilometer" | "square kilometers" => {
            "km2"
        }
        "square inch" | "square inches" => "in2",
        "square foot" | "square feet" => "ft2",
        "hectare" | "hectares" => "ha",
        "acre" | "acres" => "acre",
        "millilitre" | "millilitres" | "milliliter" | "milliliters" => "ml",
        "litre" | "litres" | "liter" | "liters" => "L",
        "cubic centimetre" | "cubic centimetres" | "cubic centimeter" | "cubic centimeters" => {
            "cm3"
        }
        "cubic metre" | "cubic metres" | "cubic meter" | "cubic meters" => "m3",
        "us gallon" | "us gallons" => "usgal",
        "imperial gallon" | "imperial gallons" => "impgal",
        "us cup" | "us cups" => "uscup",
        "us fluid ounce" | "us fluid ounces" => "usfloz",
        "imperial fluid ounce" | "imperial fluid ounces" => "impfloz",
        "metre per second" | "metres per second" | "meter per second" | "meters per second" => {
            "m/s"
        }
        "kilometre per hour"
        | "kilometres per hour"
        | "kilometer per hour"
        | "kilometers per hour" => "km/h",
        "mile per hour" | "miles per hour" => "mph",
        "knot" | "knots" => "kn",
        "nanosecond" | "nanoseconds" => "ns",
        "microsecond" | "microseconds" => "us",
        "millisecond" | "milliseconds" => "ms",
        "second" | "seconds" | "secs" => "s",
        "minute" | "minutes" | "mins" => "min",
        "hour" | "hours" | "hrs" => "h",
        "day" | "days" => "d",
        "week" | "weeks" | "wks" => "wk",
        "degree" | "degrees" => "deg",
        "radian" | "radians" => "rad",
        "turn" | "turns" => "turn",
        "celsius" | "degree celsius" | "degrees celsius" => "C",
        "fahrenheit" | "degree fahrenheit" | "degrees fahrenheit" => "F",
        "kelvin" | "kelvins" => "K",
        "bit" | "bits" => "b",
        "byte" | "bytes" => "B",
        "kilobit" | "kilobits" => "kb",
        "megabit" | "megabits" => "Mb",
        "gigabit" | "gigabits" => "Gb",
        "terabit" | "terabits" => "Tb",
        "kilobyte" | "kilobytes" => "kB",
        "megabyte" | "megabytes" => "MB",
        "gigabyte" | "gigabytes" => "GB",
        "terabyte" | "terabytes" => "TB",
        "kibibyte" | "kibibytes" => "KiB",
        "mebibyte" | "mebibytes" => "MiB",
        "gibibyte" | "gibibytes" => "GiB",
        "tebibyte" | "tebibytes" => "TiB",
        _ => name,
    }
}

fn base(name: &str) -> Option<u32> {
    match name {
        "bin" | "binary" => Some(2),
        "oct" | "octal" => Some(8),
        "dec" | "decimal" => Some(10),
        "hex" | "hexadecimal" => Some(16),
        _ => name.parse().ok(),
    }
}

fn convert_base(value: &str, source: &str, target: &str) -> Result<String, &'static str> {
    let parse_base = |name: &str| {
        base(name)
            .filter(|base| (2..=36).contains(base))
            .ok_or("bases must be between 2 and 36")
    };
    let source = parse_base(source)?;
    let target = parse_base(target)?;
    let value = i128::from_str_radix(value, source)
        .map_err(|_| "invalid integer for this base or outside the signed 128-bit range")?;
    let mut magnitude = value.unsigned_abs();
    let mut digits = Vec::new();
    loop {
        let digit = (magnitude % u128::from(target)) as u32;
        digits.push(char::from_digit(digit, target).expect("digit is less than the base"));
        magnitude /= u128::from(target);
        if magnitude == 0 {
            break;
        }
    }
    if value < 0 {
        digits.push('-');
    }
    Ok(digits.into_iter().rev().collect())
}

fn convert_color(value: &str, source: &str) -> Result<String, &'static str> {
    if source == "rgb" {
        let value = value
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
            .unwrap_or(value);
        let mut components = value.split(',');
        let mut channels = [0_u8; 3];
        for channel in &mut channels {
            *channel = components
                .next()
                .and_then(|component| component.trim().parse().ok())
                .ok_or("RGB requires three integers between 0 and 255")?;
        }
        if components.next().is_some() {
            return Err("RGB requires three integers between 0 and 255");
        }
        let [red, green, blue] = channels;
        Ok(format!("#{red:02x}{green:02x}{blue:02x}"))
    } else {
        let value = value.strip_prefix('#').unwrap_or(value);
        if !matches!(value.len(), 3 | 6) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("hex colours require 3 or 6 hexadecimal digits");
        }
        let packed = u32::from_str_radix(value, 16).expect("validated hexadecimal digits");
        let [red, green, blue] = if value.len() == 3 {
            [
                (packed >> 8) * 17,
                ((packed >> 4) & 15) * 17,
                (packed & 15) * 17,
            ]
        } else {
            [packed >> 16, (packed >> 8) & 255, packed & 255]
        };
        Ok(format!("rgb({red}, {green}, {blue})"))
    }
}

fn convert_unit(value: f64, source: &str, target: &str) -> Result<String, &'static str> {
    let temperature = |unit: &str| matches!(unit, "C" | "F" | "K");
    if temperature(source) && temperature(target) {
        let celsius = match source {
            "F" => (value - 32.0) / 1.8,
            "K" => value - 273.15,
            _ => value,
        };
        if celsius < -273.15 {
            return Err("temperature is below absolute zero");
        }
        return format_number(match target {
            "F" => celsius * 1.8 + 32.0,
            "K" => celsius + 273.15,
            _ => celsius,
        });
    }
    let (source_kind, source_scale) = unit(source).ok_or("unknown source unit")?;
    let (target_kind, target_scale) = unit(target).ok_or("unknown target unit")?;
    if source_kind != target_kind {
        return Err("units measure different quantities");
    }
    format_number(value * (source_scale / target_scale))
}

// Scales use metres, kilograms, square metres, litres, metres/second, seconds,
// radians and bytes. Regional volume units are explicit to avoid ambiguity.
fn unit(name: &str) -> Option<(&'static str, f64)> {
    Some(match name {
        "mm" => ("length", 0.001),
        "cm" => ("length", 0.01),
        "m" => ("length", 1.0),
        "km" => ("length", 1000.0),
        "in" => ("length", 0.0254),
        "ft" => ("length", 0.3048),
        "yd" => ("length", 0.9144),
        "mi" => ("length", 1609.344),
        "nmi" => ("length", 1852.0),
        "mg" => ("mass", 0.000_001),
        "g" => ("mass", 0.001),
        "kg" => ("mass", 1.0),
        "t" => ("mass", 1000.0),
        "oz" => ("mass", 0.453_592_37 / 16.0),
        "lb" | "lbs" => ("mass", 0.453_592_37),
        "st" => ("mass", 0.453_592_37 * 14.0),
        "mm2" => ("area", 0.000_001),
        "cm2" => ("area", 0.0001),
        "m2" => ("area", 1.0),
        "km2" => ("area", 1_000_000.0),
        "in2" => ("area", 0.0254 * 0.0254),
        "ft2" => ("area", 0.3048 * 0.3048),
        "ha" => ("area", 10_000.0),
        "acre" => ("area", 4_046.856_422_4),
        "ml" | "mL" | "cm3" => ("volume", 0.001),
        "l" | "L" => ("volume", 1.0),
        "m3" => ("volume", 1000.0),
        "usgal" => ("volume", 3.785_411_784),
        "impgal" => ("volume", 4.546_09),
        "uscup" => ("volume", 3.785_411_784 / 16.0),
        "usfloz" => ("volume", 3.785_411_784 / 128.0),
        "impfloz" => ("volume", 4.546_09 / 160.0),
        "m/s" => ("speed", 1.0),
        "km/h" | "kph" => ("speed", 1.0 / 3.6),
        "mph" => ("speed", 1609.344 / 3600.0),
        "kn" => ("speed", 1852.0 / 3600.0),
        "ns" => ("time", 1e-9),
        "us" => ("time", 1e-6),
        "ms" => ("time", 0.001),
        "s" | "sec" => ("time", 1.0),
        "min" => ("time", 60.0),
        "h" | "hr" => ("time", 3600.0),
        "d" => ("time", 86_400.0),
        "wk" => ("time", 604_800.0),
        "deg" => ("angle", std::f64::consts::PI / 180.0),
        "rad" => ("angle", 1.0),
        "turn" => ("angle", std::f64::consts::TAU),
        "b" => ("data", 0.125),
        "B" => ("data", 1.0),
        "kb" => ("data", 125.0),
        "Mb" => ("data", 125_000.0),
        "Gb" => ("data", 125_000_000.0),
        "Tb" => ("data", 125_000_000_000.0),
        "kB" | "KB" => ("data", 1000.0),
        "MB" => ("data", 1_000_000.0),
        "GB" => ("data", 1_000_000_000.0),
        "TB" => ("data", 1_000_000_000_000.0),
        "KiB" => ("data", 1024.0),
        "MiB" => ("data", 1_048_576.0),
        "GiB" => ("data", 1_073_741_824.0),
        "TiB" => ("data", 1_099_511_627_776.0),
        _ => return None,
    })
}
