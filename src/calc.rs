//! Arithmetic, done rather than remembered.
//!
//! A language model does sums the way it does everything else: by what the
//! answer looks like. It will tell you 384 * 517 with total confidence and be
//! out by a few thousand, and it will be *nearly* right, which is worse than
//! being obviously wrong. This is the one tool here that needs nothing outside
//! the machine, so it is offered whether or not anything else is.
//!
//! A recursive descent parser over a handful of tokens. Precedence the way
//! everyone writes it: `+ -` below `* / %` below `^`, which binds to the right
//! so that `2^3^2` is five hundred and twelve and not sixty-four.

/// Work out what an expression comes to.
pub fn evaluate(expression: &str) -> Result<String, String> {
    let tokens = scan(expression)?;
    let mut at = 0;
    let value = expr(&tokens, &mut at)?;
    if at < tokens.len() {
        return Err(format!(
            "there is something left over after {}",
            show(value)
        ));
    }
    if !value.is_finite() {
        return Err("that does not come to a number".into());
    }
    Ok(show(value))
}

/// A number the way somebody would write it down.
///
/// Binary floating point cannot hold a tenth, so `0.1 + 0.2` comes out as
/// `0.30000000000000004`, and handing that to somebody who asked what a fifth
/// of something is would be answering a question they did not ask. Twelve
/// significant figures is well inside what a double actually knows and well
/// outside what anybody typed.
fn show(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let rounded = format!("{value:.12}");
    let trimmed = rounded.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        return "0".into();
    }
    // Something enormous or tiny is better said in the notation it belongs in.
    if value.abs() >= 1e15 || (value != 0.0 && value.abs() < 1e-9) {
        return format!("{value:e}");
    }
    trimmed.to_string()
}

#[derive(Clone, PartialEq, Debug)]
enum Token {
    Number(f64),
    Name(String),
    Sign(char),
    Open,
    Close,
    Comma,
}

fn scan(text: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '0'..='9' | '.' => {
                let mut text = String::new();
                while i < chars.len() {
                    match chars[i] {
                        c @ ('0'..='9' | '.') => {
                            text.push(c);
                            i += 1;
                        }
                        // A grouping mark inside a number, which is how a
                        // number pasted out of a note is written. Underscore
                        // only: a comma between two digits is the separator in
                        // `1,250` and the one between arguments in `min(3,1)`,
                        // and there is no telling those apart. The comma is the
                        // one that has to work.
                        '_' if chars.get(i + 1).is_some_and(char::is_ascii_digit) => i += 1,
                        _ => break,
                    }
                }
                let value = text
                    .parse::<f64>()
                    .map_err(|_| format!("{text} is not a number"))?;
                out.push(Token::Number(value));
            }
            c if c.is_alphabetic() => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                out.push(Token::Name(
                    chars[start..i].iter().collect::<String>().to_lowercase(),
                ));
            }
            '+' | '-' | '*' | '/' | '%' | '^' => {
                out.push(Token::Sign(c));
                i += 1;
            }
            // The other spelling of times, which anybody copying a sum out of a
            // note is liable to have used.
            '×' => {
                out.push(Token::Sign('*'));
                i += 1;
            }
            '÷' => {
                out.push(Token::Sign('/'));
                i += 1;
            }
            '(' | '[' => {
                out.push(Token::Open);
                i += 1;
            }
            ')' | ']' => {
                out.push(Token::Close);
                i += 1;
            }
            ',' => {
                out.push(Token::Comma);
                i += 1;
            }
            other => return Err(format!("{other} is not something this can work out")),
        }
    }
    if out.is_empty() {
        return Err("there is nothing here to work out".into());
    }
    Ok(out)
}

fn expr(tokens: &[Token], at: &mut usize) -> Result<f64, String> {
    let mut left = term(tokens, at)?;
    while let Some(Token::Sign(c @ ('+' | '-'))) = tokens.get(*at) {
        let c = *c;
        *at += 1;
        let right = term(tokens, at)?;
        left = if c == '+' { left + right } else { left - right };
    }
    Ok(left)
}

fn term(tokens: &[Token], at: &mut usize) -> Result<f64, String> {
    let mut left = unary(tokens, at)?;
    while let Some(Token::Sign(c @ ('*' | '/' | '%'))) = tokens.get(*at) {
        let c = *c;
        *at += 1;
        let right = unary(tokens, at)?;
        left = match c {
            '*' => left * right,
            '/' if right == 0.0 => return Err("that divides by zero".into()),
            '/' => left / right,
            _ if right == 0.0 => return Err("that divides by zero".into()),
            _ => left % right,
        };
    }
    Ok(left)
}

/// A sign, and then whatever it is the sign of.
///
/// Looser than the power below it, so `-2^2` is minus four and not four: the
/// minus applies to the square, which is how everybody writes it and what
/// every other calculator does.
fn unary(tokens: &[Token], at: &mut usize) -> Result<f64, String> {
    match tokens.get(*at) {
        Some(Token::Sign('-')) => {
            *at += 1;
            Ok(-unary(tokens, at)?)
        }
        Some(Token::Sign('+')) => {
            *at += 1;
            unary(tokens, at)
        }
        _ => power(tokens, at),
    }
}

/// To the right, so `2^3^2` is two to the ninth and not eight squared. The
/// exponent goes back through the sign, so `2^-1` is a half.
fn power(tokens: &[Token], at: &mut usize) -> Result<f64, String> {
    let base = atom(tokens, at)?;
    if let Some(Token::Sign('^')) = tokens.get(*at) {
        *at += 1;
        let exponent = unary(tokens, at)?;
        return Ok(base.powf(exponent));
    }
    Ok(base)
}

fn atom(tokens: &[Token], at: &mut usize) -> Result<f64, String> {
    match tokens.get(*at).cloned() {
        Some(Token::Number(value)) => {
            *at += 1;
            Ok(value)
        }
        Some(Token::Open) => {
            *at += 1;
            let inner = expr(tokens, at)?;
            match tokens.get(*at) {
                Some(Token::Close) => {
                    *at += 1;
                    Ok(inner)
                }
                _ => Err("a bracket was opened and not closed".into()),
            }
        }
        Some(Token::Name(name)) => {
            *at += 1;
            if let Some(value) = constant(&name) {
                return Ok(value);
            }
            let mut args = Vec::new();
            if let Some(Token::Open) = tokens.get(*at) {
                *at += 1;
                if tokens.get(*at) != Some(&Token::Close) {
                    args.push(expr(tokens, at)?);
                    while tokens.get(*at) == Some(&Token::Comma) {
                        *at += 1;
                        args.push(expr(tokens, at)?);
                    }
                }
                match tokens.get(*at) {
                    Some(Token::Close) => *at += 1,
                    _ => return Err(format!("{name} was opened and not closed")),
                }
            }
            apply(&name, &args)
        }
        _ => Err("something is missing here".into()),
    }
}

fn constant(name: &str) -> Option<f64> {
    match name {
        "pi" => Some(std::f64::consts::PI),
        "e" => Some(std::f64::consts::E),
        "tau" => Some(std::f64::consts::TAU),
        _ => None,
    }
}

fn apply(name: &str, args: &[f64]) -> Result<f64, String> {
    let one = |what: &str| -> Result<f64, String> {
        match args {
            [x] => Ok(*x),
            _ => Err(format!("{what} takes one number")),
        }
    };
    match name {
        "sqrt" => {
            let x = one("sqrt")?;
            (x >= 0.0)
                .then(|| x.sqrt())
                .ok_or_else(|| "there is no square root of a negative number".into())
        }
        "abs" => Ok(one("abs")?.abs()),
        "round" => Ok(one("round")?.round()),
        "floor" => Ok(one("floor")?.floor()),
        "ceil" => Ok(one("ceil")?.ceil()),
        "ln" => Ok(one("ln")?.ln()),
        "log" => match args {
            [x] => Ok(x.log10()),
            [x, base] => Ok(x.log(*base)),
            _ => Err("log takes a number, and optionally a base".into()),
        },
        "exp" => Ok(one("exp")?.exp()),
        "sin" => Ok(one("sin")?.sin()),
        "cos" => Ok(one("cos")?.cos()),
        "tan" => Ok(one("tan")?.tan()),
        "min" => args
            .iter()
            .copied()
            .reduce(f64::min)
            .ok_or_else(|| "min needs some numbers".into()),
        "max" => args
            .iter()
            .copied()
            .reduce(f64::max)
            .ok_or_else(|| "max needs some numbers".into()),
        "pow" => match args {
            [x, y] => Ok(x.powf(*y)),
            _ => Err("pow takes two numbers".into()),
        },
        other => Err(format!("{other} is not something this knows")),
    }
}
