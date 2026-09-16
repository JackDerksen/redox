mod conversions;

pub(super) const CONVERT_COMMAND: &str = "convert";

pub(super) fn expression(command: &str) -> Option<&str> {
    let expression = command.trim();
    (expression.starts_with(|character: char| {
        character.is_ascii_digit() || matches!(character, '.' | '(' | '+' | '-')
    }) || expression.split_whitespace().next() == Some(CONVERT_COMMAND))
    .then_some(expression)
}

pub(super) fn evaluate(expression: &str) -> Result<String, &'static str> {
    if expression.len() > 4096 {
        return Err("expression is too long");
    }
    let expression = expression.trim();
    if expression.split_whitespace().next() == Some(CONVERT_COMMAND) {
        let expression = expression[CONVERT_COMMAND.len()..].trim();
        let (position, _) = expression
            .match_indices("to")
            .find(|(position, _)| {
                expression[..*position].ends_with(char::is_whitespace)
                    && expression[*position + 2..].starts_with(char::is_whitespace)
            })
            .ok_or("usage: convert value source to target")?;
        return conversions::evaluate(
            expression[..position].trim(),
            expression[position + 2..].trim(),
        );
    }
    format_number(evaluate_number(expression)?)
}

fn evaluate_number(expression: &str) -> Result<f64, &'static str> {
    let mut parser = Parser {
        remaining: expression,
    };
    let value = parser.expression(0, 0)?;
    if !parser.remaining.trim().is_empty() {
        return Err("unexpected character");
    }
    Ok(value)
}

fn format_number(value: f64) -> Result<String, &'static str> {
    if !value.is_finite() {
        return Err("result is not finite");
    }
    if value == 0.0 {
        return Ok("0".to_string());
    }
    // Keep whole numbers intact and suppress floating-point noise in fractions.
    let value = if value.fract() == 0.0 {
        value
    } else {
        format!("{value:.14e}")
            .parse::<f64>()
            .expect("formatted finite number")
    };
    Ok(value.to_string())
}

struct Parser<'a> {
    remaining: &'a str,
}

impl Parser<'_> {
    fn expression(&mut self, minimum_precedence: u8, depth: usize) -> Result<f64, &'static str> {
        if depth >= 64 {
            return Err("expression is nested too deeply");
        }
        self.remaining = self.remaining.trim_start();
        let mut value = match self.remaining.as_bytes().first() {
            Some(b'+' | b'-') => {
                let negative = self.remaining.starts_with('-');
                self.remaining = &self.remaining[1..];
                let value = self.expression(3, depth + 1)?;
                if negative { -value } else { value }
            }
            Some(b'(') => {
                self.remaining = &self.remaining[1..];
                let value = self.expression(0, depth + 1)?;
                self.remaining = self
                    .remaining
                    .trim_start()
                    .strip_prefix(')')
                    .ok_or("missing closing parenthesis")?;
                value
            }
            _ => self.number()?,
        };
        loop {
            self.remaining = self.remaining.trim_start();
            let (operator, precedence) = match self.remaining.as_bytes().first() {
                Some(operator @ (b'+' | b'-')) => (*operator, 1),
                Some(operator @ (b'*' | b'x' | b'/' | b'%')) => (*operator, 2),
                Some(b'^') => (b'^', 4),
                _ => break,
            };
            if precedence < minimum_precedence {
                break;
            }
            self.remaining = &self.remaining[1..];
            let right = self.expression(precedence + u8::from(operator != b'^'), depth + 1)?;
            if matches!(operator, b'/' | b'%') && right == 0.0 {
                return Err("division by zero");
            }
            value = match operator {
                b'+' => value + right,
                b'-' => value - right,
                b'*' | b'x' => value * right,
                b'/' => value / right,
                b'%' => value % right,
                b'^' => value.powf(right),
                _ => unreachable!(),
            };
            if !value.is_finite() {
                return Err("result is not finite");
            }
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<f64, &'static str> {
        let bytes = self.remaining.as_bytes();
        let mut end = 0;
        while bytes
            .get(end)
            .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
        {
            end += 1;
        }
        if end == 0 {
            return Err("expected a number");
        }
        if matches!(bytes.get(end), Some(b'e' | b'E')) {
            end += 1;
            if matches!(bytes.get(end), Some(b'+' | b'-')) {
                end += 1;
            }
            while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
        }
        let value = self.remaining[..end]
            .parse::<f64>()
            .map_err(|_| "invalid number")?;
        self.remaining = &self.remaining[end..];
        if !value.is_finite() {
            return Err("number is too large");
        }
        // Beyond 2^53 - 1 (max safe int), parsing can silently round distinct integers to the same f64.
        if value > 9_007_199_254_740_991.0 {
            return Err("number exceeds the exact integer range");
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_cover_bases_units_and_colors_without_claiming_commands() {
        for (input, expected) in [
            ("90 minutes to seconds", "5400"),
            ("1.5 hours to minutes", "90"),
            ("2 weeks to days", "14"),
            ("1000 nanoseconds to microseconds", "1"),
            ("1 second to milliseconds", "1000"),
            ("180 degrees to radians", "3.14159265358979"),
            ("1 turn to radians", "6.28318530717959"),
            ("5 kilograms to pounds", "11.0231131092439"),
            ("(5 + 2) kilograms to grams", "7000"),
            ("1 metric tonne to kilograms", "1000"),
            ("16 ounces to pound", "1"),
            ("12 inches to feet", "1"),
            ("1 yard to feet", "3"),
            ("1 metre to centimeters", "100"),
            ("1 nautical mile to METRES", "1852"),
            ("2 square metres to square centimetres", "20000"),
            ("144 square inches to square feet", "1"),
            ("1 hectare to square meters", "10000"),
            ("1 US gallon to US cups", "16"),
            ("1 Imperial gallon to litres", "4.54609"),
            ("1 US fluid ounce to millilitres", "29.5735295625"),
            ("1 cubic meter to litres", "1000"),
            ("1000 cubic centimetres to litre", "1"),
            (
                "100 kilometres per hour to miles per hour",
                "62.1371192237334",
            ),
            ("1 knot to kilometers per hour", "1.852"),
            ("0 Celsius to degrees Fahrenheit", "32"),
            ("-40 degrees Fahrenheit to celsius", "-40"),
            ("273.15 kelvins to degree Celsius", "0"),
            ("1 megabyte to bytes", "1000000"),
            ("1 Mebibyte to bytes", "1048576"),
            ("8 MEGABITS to megabytes", "1"),
            ("1 Mb to MB", "0.125"),
            ("100 binary to decimal", "4"),
            ("111 bin to oct", "7"),
            ("77 octal to dec", "63"),
            ("255 decimal to hex", "ff"),
            ("FF hex to decimal", "255"),
            ("FF hexadecimal to binary", "11111111"),
            ("e hex to decimal", "14"),
            ("rgb(255, 128, 0) to hexadecimal", "#ff8000"),
            ("FF8000 hexadecimal to rgb", "rgb(255, 128, 0)"),
            ("100 km/h to mph", "62.1371192237334"),
            ("5 kg to lbs", "11.0231131092439"),
            ("(5 + 2) kg to g", "7000"),
            ("  1.5\t h  to  min  ", "90"),
            ("-40 C to F", "-40"),
            ("1e3 m to km", "1"),
            ("100 2 to 10", "4"),
            ("FF 16 to 10", "255"),
            ("to 36 to 10", "1068"),
            ("9007199254740993 10 to 16", "20000000000001"),
            ("FF8000 hex to rgb", "rgb(255, 128, 0)"),
            ("#abc hex to rgb", "rgb(170, 187, 204)"),
            ("(255, 128, 0) rgb to hex", "#ff8000"),
            ("100_2 to 10", "4"),
            ("100_2 to _10", "4"),
            ("1_kg to _g", "1000"),
            ("#fff to _rgb", "rgb(255, 255, 255)"),
            ("FF_16 to 10", "255"),
            ("255_10 to 16", "ff"),
            ("-ff_16 to 2", "-11111111"),
            ("0_10 to 36", "0"),
            ("z_36 to 10", "35"),
            ("to_36 to 10", "1068"),
            ("9007199254740993_10 to 16", "20000000000001"),
            ("20000000000001_16 to 10", "9007199254740993"),
            (
                "-80000000000000000000000000000000_16 to 10",
                "-170141183460469231731687303715884105728",
            ),
            ("1_mi to km", "1.609344"),
            ("12_in to ft", "1"),
            ("5_kg to lbs", "11.0231131092439"),
            ("16_oz to lb", "1"),
            ("(5+2)_kg to g", "7000"),
            ("1_m2 to cm2", "10000"),
            ("1_acre to m2", "4046.8564224"),
            ("1_usgal to L", "3.785411784"),
            ("1_impgal to L", "4.54609"),
            ("1_uscup to usfloz", "8"),
            ("100_km/h to mph", "62.1371192237334"),
            ("1_kn to km/h", "1.852"),
            ("90_min to h", "1.5"),
            ("1.5_h\tto\ts", "5400"),
            ("  1_day   to   _h  ", "24"),
            ("180_deg to rad", "3.14159265358979"),
            ("1_turn to deg", "360"),
            ("0_C to F", "32"),
            ("212_F to C", "100"),
            ("-40_F to C", "-40"),
            ("273.15_K to C", "0"),
            ("-273.15_C to K", "0"),
            ("-459.67_F to K", "0"),
            ("1_MB to B", "1000000"),
            ("1_MiB to B", "1048576"),
            ("1_GiB to MB", "1073.741824"),
            ("8_Mb to MB", "1"),
            ("8_bit to B", "1"),
            ("rgb(255, 128, 0) to hex", "#ff8000"),
            ("255,128,0_rgb to hex", "#ff8000"),
            ("(0, 0, 1)_rgb to hex", "#000001"),
            ("#ff8000 to rgb", "rgb(255, 128, 0)"),
            ("FF8000_hex to rgb", "rgb(255, 128, 0)"),
            ("#AbC to rgb", "rgb(170, 187, 204)"),
        ] {
            let input = format!("convert {input}");
            assert_eq!(expression(&input), Some(input.trim()), "{input}");
            assert_eq!(evaluate(&input), Ok(expected.to_string()), "{input}");
            let spaced = input.replace('_', " ");
            assert_eq!(expression(&spaced), Some(spaced.trim()), "{spaced}");
            assert_eq!(evaluate(&spaced), Ok(expected.to_string()), "{spaced}");
        }
        for input in [
            "e 16 to 10",
            "ex 36 to 10",
            "FF hex to decimal",
            "#fff to rgb",
            "rgb(255,0,0) to hex",
            "convert-file",
            "e file_16 to 10",
            "config",
            "colorscheme default",
            "macros",
            "quit",
            "do_stuff",
        ] {
            assert_eq!(expression(input), None, "{input}");
        }
        for input in [
            "",
            "1 minutes to metres",
            "1 gallons to litres",
            "1 ton to kilograms",
            "1 metress to centimetres",
            "1 square radians to degrees",
            "1 mB to MB",
            "1 ML to L",
            "2 binary to decimal",
            "FF hex to kg",
            "1 kg to decimal",
            "1.5 decimal to binary",
            "1 kg to m",
            "1 unknown to g",
            "1 kg extra to g",
            "1 to kg",
            "1 kg to",
            "1 kg to g trailing",
            "FF 2 to 10",
            "9007199254740993 kg to lbs",
            "2_2 to 10",
            "1_1 to 10",
            "1_10 to 37",
            "1.5_10 to 2",
            "170141183460469231731687303715884105728_10 to 2",
            "-170141183460469231731687303715884105729_10 to 2",
            "1_kg to m",
            "1_gal to L",
            "1_USD to CAD",
            "1_kg to ",
            "1_kg to g trailing",
            "1_kg to g to lb",
            "1/0_kg to g",
            "9007199254740993_kg to lbs",
            "(10^308)_kg to mg",
            "-1_K to C",
            "rgb(256,0,0) to hex",
            "rgb(-1,0,0) to hex",
            "rgb(1.5,0,0) to hex",
            "rgb(0,0) to hex",
            "rgb(0,0,0,0) to hex",
            "#ff to rgb",
            "#zzzzzz to rgb",
            "#猫猫 to rgb",
            "#ffffffff to rgb",
            "255,0,0_rgb to kg",
        ] {
            assert!(evaluate(&format!("convert {input}")).is_err(), "{input}");
        }
        assert_eq!(
            evaluate("convert\t100\tkm/h to mph"),
            Ok("62.1371192237334".to_string())
        );
        assert!(evaluate("100 km/h to mph").is_err());
    }

    #[test]
    fn integer_literals_must_stay_within_f64_safe_range() {
        assert_eq!(
            evaluate("9007199254740991"),
            Ok("9007199254740991".to_string())
        );
        for input in [
            "9007199254740993",
            "-9007199254740993",
            "9007199254740993.0",
            "9.007199254740993e15",
        ] {
            assert_eq!(
                evaluate(input),
                Err("number exceeds the exact integer range"),
                "{input}"
            );
        }
    }

    #[test]
    fn arithmetic_respects_precedence_and_rejects_invalid_input() {
        for (input, expected) in [
            ("5+(2*3)", "11"),
            ("5+(2x3)", "11"),
            ("2 x -3", "-6"),
            (" 1.5 * -2 ", "-3"),
            ("8/4/2", "1"),
            ("2^3^2", "512"),
            ("-2^2", "-4"),
            ("(-2)^2", "4"),
            ("2^-3", "0.125"),
            ("7%3", "1"),
            (".5+1e-2", "0.51"),
            ("0.1+0.2", "0.3"),
            ("-0", "0"),
        ] {
            assert_eq!(evaluate(input), Ok(expected.to_string()), "{input}");
        }
        for input in [
            "", "1/0", "1%0", "1+", "(2*3", "2(3)", "1..2", "1e", "2+猫", "1e309", "1e308*10",
            "(-1)^.5", "5+(2*3)=",
        ] {
            assert!(evaluate(input).is_err(), "{input}");
        }
        assert!(evaluate(&"(".repeat(100)).is_err());
        assert!(evaluate(&"1".repeat(4097)).is_err());
        assert_eq!(expression("e file="), None);
        assert_eq!(expression(""), None);
    }
}
