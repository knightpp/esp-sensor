use embedded_svc::io::{Write, WriteFmtError};
use std::fmt::Display;

#[derive(Debug)]
pub enum Error<E> {
    Write(E),
    Fmt(WriteFmtError<E>),
    StartWithUnderScore,
    ContainsNewLine,
    ContainsQuotes,
}

impl<E: std::error::Error> Display for Error<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Write(err) => write!(f, "{err:?}"),
            Self::Fmt(err) => write!(f, "{err}"),
            Self::StartWithUnderScore => write!(f, "value starts with underscore"),
            Self::ContainsNewLine => write!(f, "value contains new line"),
            Self::ContainsQuotes => write!(f, "value contains quotes"),
        }
    }
}

pub fn write<W: Write<Error = E>, E>(
    mut w: W,
    measurement: &str,
    tags: &[(&str, &str)],
    fields: &[(&str, f32)],
) -> Result<(), Error<E>> {
    validate_str(measurement)?;
    for (name, value) in tags {
        validate_str(name)?;
        validate_str(value)?;
    }
    for (field_name, _) in fields {
        validate_str(field_name)?;
    }

    w.write_all(measurement.as_bytes())?;
    for (name, value) in tags {
        w.write_fmt(format_args!(",{name}={value}"))?;
    }
    w.write_all(b" ")?;
    for (i, (name, value)) in fields.iter().enumerate() {
        if i != 0 {
            w.write_all(b",")?;
        }
        w.write_fmt(format_args!("{name}={value}"))?;
    }
    w.write(b"\n")?;

    Ok(())
}

impl<E: std::error::Error> std::error::Error for Error<E> {}

impl<E> From<E> for Error<E> {
    fn from(value: E) -> Self {
        Self::Write(value)
    }
}

impl<E> From<WriteFmtError<E>> for Error<E> {
    fn from(value: WriteFmtError<E>) -> Self {
        Self::Fmt(value)
    }
}

fn validate_str<E>(s: &str) -> Result<(), Error<E>> {
    if s.starts_with('_') {
        return Err(Error::StartWithUnderScore);
    }
    if s.contains('\n') {
        return Err(Error::ContainsNewLine);
    }
    if s.contains('"') {
        return Err(Error::ContainsQuotes);
    }

    Ok(())
}
