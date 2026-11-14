//! Subsonic error responses.

/// An error answered with HTTP 200 and a `failed` response, as the protocol wants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    /// Subsonic error code.
    pub code: u32,
    /// Message for the user of the client.
    pub message: String,
}

impl ApiError {
    fn new(code: u32, message: impl Into<String>) -> ApiError {
        ApiError {
            code,
            message: message.into(),
        }
    }

    /// 0: a generic error.
    pub fn generic(message: impl Into<String>) -> ApiError {
        ApiError::new(0, message)
    }

    /// 0: the method is not part of the supported API.
    pub fn not_implemented() -> ApiError {
        ApiError::new(0, "not implemented")
    }

    /// 10: a required parameter is missing.
    pub fn missing_parameter(name: &str) -> ApiError {
        ApiError::new(10, format!("Required parameter is missing: {name}"))
    }

    /// 40: wrong user name or password.
    pub fn wrong_credentials() -> ApiError {
        ApiError::new(40, "Wrong username or password")
    }

    /// 43: `apiKey` was sent together with another way of authenticating.
    pub fn conflicting_authentication() -> ApiError {
        ApiError::new(
            43,
            "Multiple conflicting authentication mechanisms provided",
        )
    }

    /// 44: the API key is not valid.
    pub fn invalid_api_key() -> ApiError {
        ApiError::new(44, "Invalid API key")
    }

    /// 50: the user may not do this.
    pub fn unauthorized() -> ApiError {
        ApiError::new(50, "User is not authorized for the given operation")
    }

    /// 70: the item does not exist, or the user cannot see it.
    pub fn not_found() -> ApiError {
        ApiError::new(70, "The requested data was not found")
    }

    /// Whether this is the answer to a failed login, which is delayed.
    pub fn is_auth_failure(&self) -> bool {
        matches!(self.code, 40 | 44)
    }
}

impl From<armonika_core::Error> for ApiError {
    fn from(e: armonika_core::Error) -> ApiError {
        // Details stay in the log: they can name paths and tables.
        tracing::error!(error = %e, "request failed");
        ApiError::generic("internal error")
    }
}
