use super::*;

pub trait ResultZipExt<T, E> {
    fn zip<U>(self, other: Result<U, E>) -> Result<(T, U), E>;
}

impl<T, E> ResultZipExt<T, E> for Result<T, E> {
    fn zip<U>(self, other: Result<U, E>) -> Result<(T, U), E> {
        match (self, other) {
            (Ok(one), Ok(other)) => Ok((one, other)),
            (Ok(_), Err(message)) => Err(message),
            (Err(message), Ok(_)) => Err(message),
            (Err(message), Err(_)) => Err(message),
        }
    }
}
