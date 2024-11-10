use std::{error::Error, fmt::Display};

#[derive(Debug)]
pub struct ServiceError(pub String);

impl Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl Error for ServiceError {}

impl ServiceError {
    pub fn from(e: &dyn Error) -> Self {
        ServiceError(e.to_string())
    }
}
