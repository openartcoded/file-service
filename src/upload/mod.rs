pub mod domain;
pub mod imagemagick;
pub mod routes;
pub mod service;
pub mod soffice;

pub enum ConvertType {
    Png,
    // Pdf,
}
impl ConvertType {
    fn to_str(&self) -> &str {
        match self {
            ConvertType::Png => "png",
            //  ConvertType::Pdf => "pdf",
        }
    }
}
