pub(super) fn expression(command: &str) -> Option<&str> {
    let expression = command.trim();
    expression
        .starts_with(|character: char| {
            character.is_ascii_digit() || matches!(character, '.' | '(' | '+' | '-')
        })
        .then_some(expression)
}

pub(super) fn evaluate(expression: &str) -> Result<String, &'static str> {
    if expression.len() > 4096 {
        return Err("expression is too long");
    }
    let mut parser = Parser {
        remaining: expression,
    };
    let value = parser.expression(0, 0)?;
    if !parser.remaining.trim().is_empty() {
        return Err("unexpected character");
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
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
