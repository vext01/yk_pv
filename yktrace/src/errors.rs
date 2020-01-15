use std::fmt::{self, Display, Formatter};

#[derive(Debug)]
/// Reasons that a trace can be invalidated.
pub enum InvalidTraceError {
    /// There is no SIR for the location in the trace.
    /// The string inside is FIXME
    NoSir(String),
    /// Something went wrong in the compiler's tracing code
    InternalError
}

impl InvalidTraceError {
    /// A helper function to create a `InvalidTraceError::NoSir`.
    pub fn no_sir(def_id: &str) -> Self {
        return InvalidTraceError::NoSir(String::from(def_id))
    }
}

impl Display for InvalidTraceError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            InvalidTraceError::NoSir(symbol_name) =>
                write!(f, "No SIR for location: {}", symbol_name),
            InvalidTraceError::InternalError => write!(f, "Internal tracing error")
        }
    }
}
