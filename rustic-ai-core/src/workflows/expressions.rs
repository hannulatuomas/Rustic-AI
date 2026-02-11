use regex::Regex;
use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExpressionError {
    #[error("expression parse error: {0}")]
    Parse(String),
    #[error("expression evaluation error: {0}")]
    Evaluation(String),
}

#[derive(Debug, Clone)]
enum UnaryOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, Copy)]
enum BinaryOp {
    Or,
    And,
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    Contains,
    Matches,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone)]
enum Expr {
    Root,
    Variable(String),
    Literal(Value),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },
    Access {
        target: Box<Expr>,
        segment: String,
    },
    Index {
        target: Box<Expr>,
        index: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone)]
enum Token {
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    Comma,
    Dollar,
    Not,
    Plus,
    Minus,
    Star,
    Slash,
    EqEq,
    NotEq,
    Gt,
    Gte,
    Lt,
    Lte,
    AndAnd,
    OrOr,
    Contains,
    Matches,
    Identifier(String),
    Number(f64),
    String(String),
    True,
    False,
    Null,
}

#[derive(Debug, Clone, Copy)]
pub struct EvaluationOptions {
    pub max_length: usize,
    pub max_depth: usize,
}

impl Default for EvaluationOptions {
    fn default() -> Self {
        Self {
            max_length: 8_192,
            max_depth: 64,
        }
    }
}

pub fn evaluate_expression(
    expression: &str,
    outputs: &BTreeMap<String, Value>,
) -> Result<Value, ExpressionError> {
    evaluate_expression_with_locals_and_options(
        expression,
        outputs,
        &BTreeMap::new(),
        EvaluationOptions::default(),
    )
}

pub fn evaluate_expression_with_locals(
    expression: &str,
    outputs: &BTreeMap<String, Value>,
    locals: &BTreeMap<String, Value>,
) -> Result<Value, ExpressionError> {
    evaluate_expression_with_locals_and_options(
        expression,
        outputs,
        locals,
        EvaluationOptions::default(),
    )
}

pub fn evaluate_expression_with_options(
    expression: &str,
    outputs: &BTreeMap<String, Value>,
    options: EvaluationOptions,
) -> Result<Value, ExpressionError> {
    evaluate_expression_with_locals_and_options(expression, outputs, &BTreeMap::new(), options)
}

pub fn evaluate_expression_with_locals_and_options(
    expression: &str,
    outputs: &BTreeMap<String, Value>,
    locals: &BTreeMap<String, Value>,
    options: EvaluationOptions,
) -> Result<Value, ExpressionError> {
    if expression.len() > options.max_length {
        return Err(ExpressionError::Parse(format!(
            "expression exceeds max length {}",
            options.max_length
        )));
    }

    let tokens = tokenize(expression)?;
    let mut parser = Parser::new(tokens);
    let expr = parser.parse_expression()?;
    parser.expect_end()?;
    let depth = expression_depth(&expr);
    if depth > options.max_depth {
        return Err(ExpressionError::Parse(format!(
            "expression depth {} exceeds max depth {}",
            depth, options.max_depth
        )));
    }

    let evaluator = Evaluator::new(outputs, locals);
    evaluator.eval(&expr)
}

fn expression_depth(expr: &Expr) -> usize {
    match expr {
        Expr::Root | Expr::Variable(_) | Expr::Literal(_) => 1,
        Expr::Unary { expr, .. } => 1 + expression_depth(expr),
        Expr::Binary { left, right, .. } => 1 + expression_depth(left).max(expression_depth(right)),
        Expr::Access { target, .. } => 1 + expression_depth(target),
        Expr::Index { target, index } => 1 + expression_depth(target).max(expression_depth(index)),
        Expr::Call { args, .. } => {
            let args_depth = args.iter().map(expression_depth).max().unwrap_or(0);
            1 + args_depth
        }
    }
}

pub fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Bool(v) => *v,
        Value::Number(v) => v.as_f64().map(|n| n != 0.0).unwrap_or(false),
        Value::String(v) => !v.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
        Value::Null => false,
    }
}

fn tokenize(input: &str) -> Result<Vec<Token>, ExpressionError> {
    let mut chars = input.char_indices().peekable();
    let mut tokens = Vec::new();

    while let Some((idx, ch)) = chars.peek().copied() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }

        if ch.is_ascii_digit() {
            let start = idx;
            chars.next();
            while let Some((_, c)) = chars.peek().copied() {
                if c.is_ascii_digit() || c == '.' {
                    chars.next();
                } else {
                    break;
                }
            }
            let end = chars.peek().map(|(i, _)| *i).unwrap_or(input.len());
            let text = &input[start..end];
            let number = text
                .parse::<f64>()
                .map_err(|err| ExpressionError::Parse(format!("invalid number '{text}': {err}")))?;
            tokens.push(Token::Number(number));
            continue;
        }

        if ch == '\'' || ch == '"' {
            let quote = ch;
            chars.next();
            let mut value = String::new();
            let mut escaped = false;
            let mut terminated = false;

            for (_, c) in chars.by_ref() {
                if escaped {
                    let translated = match c {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '\\' => '\\',
                        '\'' => '\'',
                        '"' => '"',
                        other => other,
                    };
                    value.push(translated);
                    escaped = false;
                    continue;
                }

                if c == '\\' {
                    escaped = true;
                    continue;
                }
                if c == quote {
                    terminated = true;
                    break;
                }
                value.push(c);
            }

            if !terminated {
                return Err(ExpressionError::Parse(
                    "unterminated string literal".to_owned(),
                ));
            }
            tokens.push(Token::String(value));
            continue;
        }

        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = idx;
            chars.next();
            while let Some((_, c)) = chars.peek().copied() {
                if c.is_ascii_alphanumeric() || c == '_' {
                    chars.next();
                } else {
                    break;
                }
            }
            let end = chars.peek().map(|(i, _)| *i).unwrap_or(input.len());
            let ident = &input[start..end];
            let token = match ident {
                "true" => Token::True,
                "false" => Token::False,
                "null" => Token::Null,
                "contains" => Token::Contains,
                "matches" => Token::Matches,
                _ => Token::Identifier(ident.to_owned()),
            };
            tokens.push(token);
            continue;
        }

        let token = match ch {
            '(' => Token::LParen,
            ')' => Token::RParen,
            '[' => Token::LBracket,
            ']' => Token::RBracket,
            '.' => Token::Dot,
            ',' => Token::Comma,
            '$' => Token::Dollar,
            '+' => Token::Plus,
            '-' => Token::Minus,
            '*' => Token::Star,
            '/' => Token::Slash,
            '!' => {
                chars.next();
                if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    tokens.push(Token::NotEq);
                } else {
                    tokens.push(Token::Not);
                }
                continue;
            }
            '=' => {
                chars.next();
                if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    tokens.push(Token::EqEq);
                    continue;
                }
                return Err(ExpressionError::Parse(
                    "unexpected '='; use '==' for comparison".to_owned(),
                ));
            }
            '>' => {
                chars.next();
                if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    tokens.push(Token::Gte);
                } else {
                    tokens.push(Token::Gt);
                }
                continue;
            }
            '<' => {
                chars.next();
                if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    tokens.push(Token::Lte);
                } else {
                    tokens.push(Token::Lt);
                }
                continue;
            }
            '&' => {
                chars.next();
                if matches!(chars.peek(), Some((_, '&'))) {
                    chars.next();
                    tokens.push(Token::AndAnd);
                    continue;
                }
                return Err(ExpressionError::Parse(
                    "unexpected '&'; use '&&'".to_owned(),
                ));
            }
            '|' => {
                chars.next();
                if matches!(chars.peek(), Some((_, '|'))) {
                    chars.next();
                    tokens.push(Token::OrOr);
                    continue;
                }
                return Err(ExpressionError::Parse(
                    "unexpected '|'; use '||'".to_owned(),
                ));
            }
            other => {
                return Err(ExpressionError::Parse(format!(
                    "unexpected character '{}'",
                    other
                )));
            }
        };

        chars.next();
        tokens.push(token);
    }

    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, index: 0 }
    }

    fn parse_expression(&mut self) -> Result<Expr, ExpressionError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, ExpressionError> {
        let mut left = self.parse_and()?;
        while self.match_token(|t| matches!(t, Token::OrOr)) {
            let right = self.parse_and()?;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinaryOp::Or,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ExpressionError> {
        let mut left = self.parse_comparison()?;
        while self.match_token(|t| matches!(t, Token::AndAnd)) {
            let right = self.parse_comparison()?;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinaryOp::And,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ExpressionError> {
        let mut left = self.parse_additive()?;
        while let Some(op) = self.match_binary_comparison_op() {
            let right = self.parse_additive()?;
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, ExpressionError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = if self.match_token(|t| matches!(t, Token::Plus)) {
                Some(BinaryOp::Add)
            } else if self.match_token(|t| matches!(t, Token::Minus)) {
                Some(BinaryOp::Sub)
            } else {
                None
            };

            let Some(op) = op else {
                break;
            };
            let right = self.parse_multiplicative()?;
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ExpressionError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = if self.match_token(|t| matches!(t, Token::Star)) {
                Some(BinaryOp::Mul)
            } else if self.match_token(|t| matches!(t, Token::Slash)) {
                Some(BinaryOp::Div)
            } else {
                None
            };

            let Some(op) = op else {
                break;
            };
            let right = self.parse_unary()?;
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ExpressionError> {
        if self.match_token(|t| matches!(t, Token::Not)) {
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(self.parse_unary()?),
            });
        }
        if self.match_token(|t| matches!(t, Token::Minus)) {
            return Ok(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(self.parse_unary()?),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, ExpressionError> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.match_token(|t| matches!(t, Token::Dot)) {
                let Some(Token::Identifier(segment)) = self.peek().cloned() else {
                    return Err(ExpressionError::Parse(
                        "expected identifier after '.'".to_owned(),
                    ));
                };
                self.index += 1;
                expr = Expr::Access {
                    target: Box::new(expr),
                    segment,
                };
                continue;
            }

            if self.match_token(|t| matches!(t, Token::LBracket)) {
                let index = self.parse_expression()?;
                self.consume(|t| matches!(t, Token::RBracket), "expected ']' after index")?;
                expr = Expr::Index {
                    target: Box::new(expr),
                    index: Box::new(index),
                };
                continue;
            }

            break;
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, ExpressionError> {
        let Some(token) = self.peek().cloned() else {
            return Err(ExpressionError::Parse(
                "unexpected end of expression".to_owned(),
            ));
        };

        match token {
            Token::Dollar => {
                self.index += 1;
                Ok(Expr::Root)
            }
            Token::True => {
                self.index += 1;
                Ok(Expr::Literal(Value::Bool(true)))
            }
            Token::False => {
                self.index += 1;
                Ok(Expr::Literal(Value::Bool(false)))
            }
            Token::Null => {
                self.index += 1;
                Ok(Expr::Literal(Value::Null))
            }
            Token::Number(v) => {
                self.index += 1;
                let number = Number::from_f64(v).ok_or_else(|| {
                    ExpressionError::Parse(format!("invalid finite number literal '{v}'"))
                })?;
                Ok(Expr::Literal(Value::Number(number)))
            }
            Token::String(v) => {
                self.index += 1;
                Ok(Expr::Literal(Value::String(v)))
            }
            Token::Identifier(name) => {
                self.index += 1;
                if self.match_token(|t| matches!(t, Token::LParen)) {
                    let mut args = Vec::new();
                    if !self.match_token(|t| matches!(t, Token::RParen)) {
                        loop {
                            args.push(self.parse_expression()?);
                            if self.match_token(|t| matches!(t, Token::RParen)) {
                                break;
                            }
                            self.consume(
                                |t| matches!(t, Token::Comma),
                                "expected ',' between function arguments",
                            )?;
                        }
                    }
                    Ok(Expr::Call { name, args })
                } else {
                    Ok(Expr::Variable(name))
                }
            }
            Token::LParen => {
                self.index += 1;
                let expr = self.parse_expression()?;
                self.consume(
                    |t| matches!(t, Token::RParen),
                    "expected ')' after expression",
                )?;
                Ok(expr)
            }
            _ => Err(ExpressionError::Parse(format!(
                "unexpected token '{token:?}'"
            ))),
        }
    }

    fn match_binary_comparison_op(&mut self) -> Option<BinaryOp> {
        let token = self.peek()?.clone();
        let op = match token {
            Token::EqEq => BinaryOp::Eq,
            Token::NotEq => BinaryOp::Neq,
            Token::Gt => BinaryOp::Gt,
            Token::Gte => BinaryOp::Gte,
            Token::Lt => BinaryOp::Lt,
            Token::Lte => BinaryOp::Lte,
            Token::Contains => BinaryOp::Contains,
            Token::Matches => BinaryOp::Matches,
            _ => return None,
        };
        self.index += 1;
        Some(op)
    }

    fn consume<F>(&mut self, predicate: F, msg: &str) -> Result<(), ExpressionError>
    where
        F: FnOnce(&Token) -> bool,
    {
        let token = self
            .peek()
            .ok_or_else(|| ExpressionError::Parse(msg.to_owned()))?;
        if predicate(token) {
            self.index += 1;
            Ok(())
        } else {
            Err(ExpressionError::Parse(msg.to_owned()))
        }
    }

    fn match_token<F>(&mut self, predicate: F) -> bool
    where
        F: FnOnce(&Token) -> bool,
    {
        if let Some(token) = self.peek() {
            if predicate(token) {
                self.index += 1;
                return true;
            }
        }
        false
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.index)
    }

    fn expect_end(&self) -> Result<(), ExpressionError> {
        if self.peek().is_some() {
            return Err(ExpressionError::Parse(
                "unexpected trailing tokens".to_owned(),
            ));
        }
        Ok(())
    }
}

struct Evaluator<'a> {
    outputs: &'a BTreeMap<String, Value>,
    locals: &'a BTreeMap<String, Value>,
}

impl<'a> Evaluator<'a> {
    fn new(outputs: &'a BTreeMap<String, Value>, locals: &'a BTreeMap<String, Value>) -> Self {
        Self { outputs, locals }
    }

    fn eval(&self, expr: &Expr) -> Result<Value, ExpressionError> {
        match expr {
            Expr::Root => Ok(self.root_value()),
            Expr::Variable(name) => Ok(self.resolve_variable(name)),
            Expr::Literal(v) => Ok(v.clone()),
            Expr::Unary { op, expr } => {
                let value = self.eval(expr)?;
                match op {
                    UnaryOp::Not => Ok(Value::Bool(!is_truthy(&value))),
                    UnaryOp::Neg => {
                        let number = to_number(&value).ok_or_else(|| {
                            ExpressionError::Evaluation(format!(
                                "cannot negate non-number value: {value}"
                            ))
                        })?;
                        Ok(number_to_value(-number))
                    }
                }
            }
            Expr::Binary { left, op, right } => {
                let left = self.eval(left)?;
                let right = self.eval(right)?;
                self.eval_binary(left, *op, right)
            }
            Expr::Access { target, segment } => {
                let value = self.eval(target)?;
                Ok(get_segment(&value, segment))
            }
            Expr::Index { target, index } => {
                let value = self.eval(target)?;
                let index = self.eval(index)?;
                Ok(match (value, index) {
                    (Value::Array(items), Value::Number(index)) => index
                        .as_u64()
                        .and_then(|idx| items.get(idx as usize).cloned())
                        .unwrap_or(Value::Null),
                    (Value::Object(map), Value::String(key)) => {
                        map.get(&key).cloned().unwrap_or(Value::Null)
                    }
                    _ => Value::Null,
                })
            }
            Expr::Call { name, args } => {
                let evaluated = args
                    .iter()
                    .map(|arg| self.eval(arg))
                    .collect::<Result<Vec<_>, _>>()?;
                self.call_function(name, &evaluated)
            }
        }
    }

    fn eval_binary(
        &self,
        left: Value,
        op: BinaryOp,
        right: Value,
    ) -> Result<Value, ExpressionError> {
        match op {
            BinaryOp::Or => Ok(Value::Bool(is_truthy(&left) || is_truthy(&right))),
            BinaryOp::And => Ok(Value::Bool(is_truthy(&left) && is_truthy(&right))),
            BinaryOp::Eq => Ok(Value::Bool(left == right)),
            BinaryOp::Neq => Ok(Value::Bool(left != right)),
            BinaryOp::Gt | BinaryOp::Gte | BinaryOp::Lt | BinaryOp::Lte => {
                compare_values(&left, &right, op)
            }
            BinaryOp::Contains => Ok(Value::Bool(contains_value(&left, &right))),
            BinaryOp::Matches => {
                let text = as_string(&left).ok_or_else(|| {
                    ExpressionError::Evaluation("left side of matches must be string".to_owned())
                })?;
                let pattern = as_string(&right).ok_or_else(|| {
                    ExpressionError::Evaluation(
                        "right side of matches must be regex string".to_owned(),
                    )
                })?;
                let regex = Regex::new(&pattern).map_err(|err| {
                    ExpressionError::Evaluation(format!("invalid regex '{pattern}': {err}"))
                })?;
                Ok(Value::Bool(regex.is_match(&text)))
            }
            BinaryOp::Add => {
                if let (Some(a), Some(b)) = (to_number(&left), to_number(&right)) {
                    return Ok(number_to_value(a + b));
                }
                Ok(Value::String(format!(
                    "{}{}",
                    value_to_string(&left),
                    value_to_string(&right)
                )))
            }
            BinaryOp::Sub => {
                let (a, b) = numbers_pair(&left, &right, "-")?;
                Ok(number_to_value(a - b))
            }
            BinaryOp::Mul => {
                let (a, b) = numbers_pair(&left, &right, "*")?;
                Ok(number_to_value(a * b))
            }
            BinaryOp::Div => {
                let (a, b) = numbers_pair(&left, &right, "/")?;
                if b == 0.0 {
                    return Err(ExpressionError::Evaluation("division by zero".to_owned()));
                }
                Ok(number_to_value(a / b))
            }
        }
    }

    fn call_function(&self, name: &str, args: &[Value]) -> Result<Value, ExpressionError> {
        let name = name.to_ascii_lowercase();
        match name.as_str() {
            "upper" => one_string_arg(&name, args, |v| Value::String(v.to_ascii_uppercase())),
            "lower" => one_string_arg(&name, args, |v| Value::String(v.to_ascii_lowercase())),
            "trim" => one_string_arg(&name, args, |v| Value::String(v.trim().to_owned())),
            "split" => {
                let text = required_arg_as_string(&name, args, 0)?;
                let delimiter = required_arg_as_string(&name, args, 1)?;
                Ok(Value::Array(
                    text.split(&delimiter)
                        .map(|item| Value::String(item.to_owned()))
                        .collect(),
                ))
            }
            "join" => {
                let items = required_arg_as_array(&name, args, 0)?;
                let delimiter = optional_arg_as_string(args, 1).unwrap_or_default();
                let joined = items
                    .iter()
                    .map(value_to_string)
                    .collect::<Vec<_>>()
                    .join(&delimiter);
                Ok(Value::String(joined))
            }
            "replace" => {
                let text = required_arg_as_string(&name, args, 0)?;
                let from = required_arg_as_string(&name, args, 1)?;
                let to = required_arg_as_string(&name, args, 2)?;
                Ok(Value::String(text.replace(&from, &to)))
            }
            "length" | "count" => {
                let Some(value) = args.first() else {
                    return Err(ExpressionError::Evaluation(format!(
                        "function '{}' expects at least 1 argument",
                        name
                    )));
                };
                Ok(number_to_value(match value {
                    Value::Array(items) => items.len() as f64,
                    Value::Object(map) => map.len() as f64,
                    Value::String(text) => text.chars().count() as f64,
                    Value::Null => 0.0,
                    _ => 1.0,
                }))
            }
            "matches" => {
                let text = required_arg_as_string(&name, args, 0)?;
                let pattern = required_arg_as_string(&name, args, 1)?;
                let regex = Regex::new(&pattern).map_err(|err| {
                    ExpressionError::Evaluation(format!("invalid regex '{pattern}': {err}"))
                })?;
                Ok(Value::Bool(regex.is_match(&text)))
            }
            "abs" => one_number_arg(&name, args, |v| number_to_value(v.abs())),
            "floor" => one_number_arg(&name, args, |v| number_to_value(v.floor())),
            "ceil" => one_number_arg(&name, args, |v| number_to_value(v.ceil())),
            "round" => one_number_arg(&name, args, |v| number_to_value(v.round())),
            "first" => {
                let items = required_arg_as_array(&name, args, 0)?;
                Ok(items.first().cloned().unwrap_or(Value::Null))
            }
            "last" => {
                let items = required_arg_as_array(&name, args, 0)?;
                Ok(items.last().cloned().unwrap_or(Value::Null))
            }
            "at" => {
                let items = required_arg_as_array(&name, args, 0)?;
                let index = required_arg_as_number(&name, args, 1)?;
                if index < 0.0 {
                    return Ok(Value::Null);
                }
                Ok(items.get(index as usize).cloned().unwrap_or(Value::Null))
            }
            "sum" => {
                let items = required_arg_as_array(&name, args, 0)?;
                let total = items.iter().filter_map(to_number).sum::<f64>();
                Ok(number_to_value(total))
            }
            "avg" => {
                let items = required_arg_as_array(&name, args, 0)?;
                let numbers = items.iter().filter_map(to_number).collect::<Vec<_>>();
                if numbers.is_empty() {
                    return Ok(Value::Null);
                }
                let total = numbers.iter().sum::<f64>();
                Ok(number_to_value(total / (numbers.len() as f64)))
            }
            "min" => {
                let items = required_arg_as_array(&name, args, 0)?;
                let min = items.iter().filter_map(to_number).reduce(f64::min);
                Ok(min.map(number_to_value).unwrap_or(Value::Null))
            }
            "max" => {
                let items = required_arg_as_array(&name, args, 0)?;
                let max = items.iter().filter_map(to_number).reduce(f64::max);
                Ok(max.map(number_to_value).unwrap_or(Value::Null))
            }
            "map" => self.map_function(args),
            "filter" => self.filter_function(args),
            "keys" => {
                let map = required_arg_as_object(&name, args, 0)?;
                Ok(Value::Array(
                    map.keys().cloned().map(Value::String).collect::<Vec<_>>(),
                ))
            }
            "values" => {
                let map = required_arg_as_object(&name, args, 0)?;
                Ok(Value::Array(map.values().cloned().collect::<Vec<_>>()))
            }
            "get" => {
                let map = required_arg_as_object(&name, args, 0)?;
                let key = required_arg_as_string(&name, args, 1)?;
                Ok(map.get(&key).cloned().unwrap_or(Value::Null))
            }
            "has" => {
                let map = required_arg_as_object(&name, args, 0)?;
                let key = required_arg_as_string(&name, args, 1)?;
                Ok(Value::Bool(map.contains_key(&key)))
            }
            "to_string" => {
                let value = args.first().cloned().unwrap_or(Value::Null);
                Ok(Value::String(value_to_string(&value)))
            }
            "to_number" => {
                let value = args.first().cloned().unwrap_or(Value::Null);
                Ok(to_number(&value)
                    .map(number_to_value)
                    .unwrap_or(Value::Null))
            }
            "to_boolean" => {
                let value = args.first().cloned().unwrap_or(Value::Null);
                Ok(Value::Bool(is_truthy(&value)))
            }
            "type" => {
                let value = args.first().cloned().unwrap_or(Value::Null);
                let kind = match value {
                    Value::Null => "null",
                    Value::Bool(_) => "boolean",
                    Value::Number(_) => "number",
                    Value::String(_) => "string",
                    Value::Array(_) => "array",
                    Value::Object(_) => "object",
                };
                Ok(Value::String(kind.to_owned()))
            }
            _ => Err(ExpressionError::Evaluation(format!(
                "unknown function '{}'",
                name
            ))),
        }
    }

    fn map_function(&self, args: &[Value]) -> Result<Value, ExpressionError> {
        let name = "map";
        let items = required_arg_as_array(name, args, 0)?;
        let expr = required_arg_as_string(name, args, 1)?;
        let var = optional_arg_as_string(args, 2).unwrap_or_else(|| "item".to_owned());
        let mut mapped = Vec::with_capacity(items.len());

        for (index, item) in items.iter().enumerate() {
            let mut locals = self.locals.clone();
            locals.insert(var.clone(), item.clone());
            locals.insert("index".to_owned(), number_to_value(index as f64));
            mapped.push(evaluate_expression_with_locals(
                &expr,
                self.outputs,
                &locals,
            )?);
        }

        Ok(Value::Array(mapped))
    }

    fn filter_function(&self, args: &[Value]) -> Result<Value, ExpressionError> {
        let name = "filter";
        let items = required_arg_as_array(name, args, 0)?;
        let expr = required_arg_as_string(name, args, 1)?;
        let var = optional_arg_as_string(args, 2).unwrap_or_else(|| "item".to_owned());
        let mut filtered = Vec::new();

        for (index, item) in items.iter().enumerate() {
            let mut locals = self.locals.clone();
            locals.insert(var.clone(), item.clone());
            locals.insert("index".to_owned(), number_to_value(index as f64));
            let passed = evaluate_expression_with_locals(&expr, self.outputs, &locals)?;
            if is_truthy(&passed) {
                filtered.push(item.clone());
            }
        }

        Ok(Value::Array(filtered))
    }

    fn root_value(&self) -> Value {
        let mut map = self
            .outputs
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Map<String, Value>>();
        for (k, v) in self.locals {
            map.insert(k.clone(), v.clone());
        }
        Value::Object(map)
    }

    fn resolve_variable(&self, name: &str) -> Value {
        if let Some(value) = self.locals.get(name) {
            return value.clone();
        }
        if let Some(value) = self.outputs.get(name) {
            return value.clone();
        }
        Value::Null
    }
}

fn get_segment(value: &Value, segment: &str) -> Value {
    match value {
        Value::Object(map) => map.get(segment).cloned().unwrap_or(Value::Null),
        Value::Array(items) => {
            if let Ok(index) = segment.parse::<usize>() {
                items.get(index).cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        _ => Value::Null,
    }
}

fn compare_values(left: &Value, right: &Value, op: BinaryOp) -> Result<Value, ExpressionError> {
    if let (Some(a), Some(b)) = (to_number(left), to_number(right)) {
        let result = match op {
            BinaryOp::Gt => a > b,
            BinaryOp::Gte => a >= b,
            BinaryOp::Lt => a < b,
            BinaryOp::Lte => a <= b,
            _ => false,
        };
        return Ok(Value::Bool(result));
    }

    if let (Some(a), Some(b)) = (as_string(left), as_string(right)) {
        let result = match op {
            BinaryOp::Gt => a > b,
            BinaryOp::Gte => a >= b,
            BinaryOp::Lt => a < b,
            BinaryOp::Lte => a <= b,
            _ => false,
        };
        return Ok(Value::Bool(result));
    }

    Err(ExpressionError::Evaluation(format!(
        "cannot compare '{}' and '{}'",
        value_to_string(left),
        value_to_string(right)
    )))
}

fn contains_value(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::String(a), Value::String(b)) => a.contains(b),
        (Value::Array(items), value) => items.iter().any(|item| item == value),
        (Value::Object(map), Value::String(key)) => map.contains_key(key),
        _ => false,
    }
}

fn numbers_pair(left: &Value, right: &Value, op: &str) -> Result<(f64, f64), ExpressionError> {
    let a = to_number(left).ok_or_else(|| {
        ExpressionError::Evaluation(format!(
            "left operand for '{}' must be a number, got {}",
            op,
            value_to_string(left)
        ))
    })?;
    let b = to_number(right).ok_or_else(|| {
        ExpressionError::Evaluation(format!(
            "right operand for '{}' must be a number, got {}",
            op,
            value_to_string(right)
        ))
    })?;
    Ok((a, b))
}

fn one_string_arg<F>(name: &str, args: &[Value], transform: F) -> Result<Value, ExpressionError>
where
    F: FnOnce(String) -> Value,
{
    let value = required_arg_as_string(name, args, 0)?;
    Ok(transform(value))
}

fn one_number_arg<F>(name: &str, args: &[Value], transform: F) -> Result<Value, ExpressionError>
where
    F: FnOnce(f64) -> Value,
{
    let value = required_arg_as_number(name, args, 0)?;
    Ok(transform(value))
}

fn required_arg_as_array<'a>(
    fn_name: &str,
    args: &'a [Value],
    index: usize,
) -> Result<&'a [Value], ExpressionError> {
    let value = args.get(index).ok_or_else(|| {
        ExpressionError::Evaluation(format!(
            "function '{}' expects argument {}",
            fn_name,
            index + 1
        ))
    })?;

    value.as_array().map(Vec::as_slice).ok_or_else(|| {
        ExpressionError::Evaluation(format!(
            "function '{}' argument {} must be array",
            fn_name,
            index + 1
        ))
    })
}

fn required_arg_as_object<'a>(
    fn_name: &str,
    args: &'a [Value],
    index: usize,
) -> Result<&'a Map<String, Value>, ExpressionError> {
    let value = args.get(index).ok_or_else(|| {
        ExpressionError::Evaluation(format!(
            "function '{}' expects argument {}",
            fn_name,
            index + 1
        ))
    })?;

    value.as_object().ok_or_else(|| {
        ExpressionError::Evaluation(format!(
            "function '{}' argument {} must be object",
            fn_name,
            index + 1
        ))
    })
}

fn required_arg_as_string(
    fn_name: &str,
    args: &[Value],
    index: usize,
) -> Result<String, ExpressionError> {
    let value = args.get(index).ok_or_else(|| {
        ExpressionError::Evaluation(format!(
            "function '{}' expects argument {}",
            fn_name,
            index + 1
        ))
    })?;

    as_string(value).ok_or_else(|| {
        ExpressionError::Evaluation(format!(
            "function '{}' argument {} must be string-compatible",
            fn_name,
            index + 1
        ))
    })
}

fn optional_arg_as_string(args: &[Value], index: usize) -> Option<String> {
    args.get(index).and_then(as_string)
}

fn required_arg_as_number(
    fn_name: &str,
    args: &[Value],
    index: usize,
) -> Result<f64, ExpressionError> {
    let value = args.get(index).ok_or_else(|| {
        ExpressionError::Evaluation(format!(
            "function '{}' expects argument {}",
            fn_name,
            index + 1
        ))
    })?;
    to_number(value).ok_or_else(|| {
        ExpressionError::Evaluation(format!(
            "function '{}' argument {} must be number-compatible",
            fn_name,
            index + 1
        ))
    })
}

fn as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(v) => Some(v.clone()),
        Value::Number(v) => Some(v.to_string()),
        Value::Bool(v) => Some(v.to_string()),
        Value::Null => None,
        _ => None,
    }
}

fn to_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(v) => v.as_f64(),
        Value::String(v) => v.trim().parse::<f64>().ok(),
        Value::Bool(v) => Some(if *v { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn number_to_value(number: f64) -> Value {
    Number::from_f64(number)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(v) => v.clone(),
        Value::Null => "null".to_owned(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}
