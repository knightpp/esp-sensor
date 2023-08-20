use core::marker::PhantomData;
use embedded_svc::io::{Write, WriteFmtError};
use std::fmt::Display;

pub struct LineBuilder<STATE, W, E>
where
    W: Write<Error = E>,
{
    w: W,
    needs_comma: bool,
    _pd: PhantomData<STATE>,
}

pub mod state {
    pub struct Measurement;
    pub struct Tag;
    pub struct Field;
    pub struct Timestamp;
}

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

pub const fn new<W: Write<Error = E>, E>(w: W) -> LineBuilder<state::Measurement, W, E> {
    LineBuilder {
        w,
        needs_comma: false,
        _pd: PhantomData,
    }
}

impl<W, E> LineBuilder<state::Measurement, W, E>
where
    W: Write<Error = E>,
{
    pub fn measurement(
        mut self,
        measurement: &str,
    ) -> Result<LineBuilder<state::Tag, W, E>, Error<E>> {
        validate_str(measurement)?;
        self.w.write(measurement.as_bytes())?;
        Ok(LineBuilder {
            w: self.w,
            needs_comma: true,
            _pd: PhantomData,
        })
    }
}

impl<W, E> LineBuilder<state::Tag, W, E>
where
    W: Write<Error = E>,
{
    #[allow(unused)]
    pub fn tag(mut self, name: &str, value: &str) -> Result<Self, Error<E>> {
        validate_str(name)?;
        validate_str(value)?;

        self.w.write_fmt(format_args!(",{name}={value}"))?;
        self.needs_comma = false;
        Ok(self)
    }

    pub fn next(mut self) -> Result<LineBuilder<state::Field, W, E>, Error<E>> {
        if self.needs_comma {
            self.w.write_all(b",")?;
        }
        self.w.write_all(b" ")?;
        Ok(LineBuilder {
            w: self.w,
            needs_comma: false,
            _pd: PhantomData,
        })
    }
}

impl<W, E> LineBuilder<state::Field, W, E>
where
    W: Write<Error = E>,
{
    pub fn field(mut self, name: &str, value: f32) -> Result<Self, Error<E>> {
        validate_str(name)?;
        if self.needs_comma {
            self.w.write_all(b",")?;
        }
        self.w.write_fmt(format_args!("{name}={value}"))?;
        self.needs_comma = true;
        Ok(self)
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn next(self) -> LineBuilder<state::Timestamp, W, E> {
        LineBuilder {
            w: self.w,
            needs_comma: false,
            _pd: PhantomData,
        }
    }
}

impl<W, E> LineBuilder<state::Timestamp, W, E>
where
    W: Write<Error = E>,
{
    #[allow(unused)]
    pub fn ts(mut self, ns: u64) -> Result<(), Error<E>> {
        self.w.write_fmt(format_args!(" {ns}"))?;
        self.build()
    }

    pub fn build(mut self) -> Result<(), Error<E>> {
        self.w.write_all(b"\n")?;
        self.w.flush()?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple() {
        let mut buf = [0_u8; 1024];

        new(&mut buf[..])
            .measurement("name")
            .unwrap()
            .tag("tag1", "value1")
            .unwrap()
            .next()
            .unwrap();
    }
}
