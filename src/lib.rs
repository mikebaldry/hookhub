use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestMessage {
    pub method: String,
    pub fullpath: String,
    pub version: Version,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

// this is annoying.

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version(u32);

impl From<actix_web::http::Version> for Version {
    fn from(value: actix_web::http::Version) -> Self {
        Version(match value {
            actix_web::http::Version::HTTP_09 => 0,
            actix_web::http::Version::HTTP_10 => 1,
            actix_web::http::Version::HTTP_11 => 2,
            actix_web::http::Version::HTTP_2 => 2,
            actix_web::http::Version::HTTP_3 => 3,
            _ => panic!("unknown version: {:?}", value),
        })
    }
}

impl From<Version> for http::Version {
    fn from(value: Version) -> Self {
        match value.0 {
            0 => http::Version::HTTP_09,
            1 => http::Version::HTTP_10,
            2 => http::Version::HTTP_11,
            3 => http::Version::HTTP_2,
            4 => http::Version::HTTP_3,
            _ => panic!("unknown version: {:?}", value),
        }
    }
}
