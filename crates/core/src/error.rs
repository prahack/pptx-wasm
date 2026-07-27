use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// The bytes are not a readable ZIP/OPC container.
    Container(String),
    /// A part the presentation cannot render without is absent.
    MissingPart(String),
    /// XML that could not be tokenised at all. Recoverable content errors never reach
    /// here — they are logged and skipped by the parsers.
    Xml(String),
    /// The package is a valid ZIP but not a presentation (e.g. a .docx).
    NotAPresentation,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Container(m) => write!(f, "cannot read .pptx container: {m}"),
            Error::MissingPart(p) => write!(f, "required part missing from package: {p}"),
            Error::Xml(m) => write!(f, "malformed XML: {m}"),
            Error::NotAPresentation => {
                write!(f, "package is a valid archive but not a presentation")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<quick_xml::Error> for Error {
    fn from(e: quick_xml::Error) -> Self {
        Error::Xml(e.to_string())
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(e: zip::result::ZipError) -> Self {
        Error::Container(e.to_string())
    }
}
