use core::marker::PhantomData;
use embedded_svc::io::{Write, WriteFmtError};

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
    pub struct Ready;
}

#[derive(Debug)]
pub enum Error<E> {
    FmtError(WriteFmtError<E>),
    Error(E),
}

impl<E> From<E> for Error<E> {
    fn from(value: E) -> Self {
        Self::Error(value)
    }
}

impl<E> From<WriteFmtError<E>> for Error<E> {
    fn from(value: WriteFmtError<E>) -> Self {
        Self::FmtError(value)
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
    pub fn tag(mut self, name: &str, value: &str) -> Result<Self, Error<E>> {
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
        if self.needs_comma {
            self.w.write_all(b",")?;
        }
        self.w.write_fmt(format_args!("{name}={value}"))?;
        self.needs_comma = true;
        Ok(self)
    }

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
    pub fn ts(mut self, ns: u64) -> Result<W, Error<E>> {
        self.w.write_fmt(format_args!(" {ns}"))?;
        self.build()
    }

    pub fn build(mut self) -> Result<W, Error<E>> {
        self.w.write_all(b"\n")?;
        self.w.flush()?;
        Ok(self.w)
    }
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
