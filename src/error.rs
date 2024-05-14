/// `tokio-listener`-specific bind errors, to be packed in [`std::io::Error::other`].
#[derive(Debug)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum BindError {
    /// Binding failed because of support of specified address type was not enabled at compile time.
    MissingCompileTimeFeature{reason: &'static str, feature: &'static str},

    /// Binding failed because of support of specified address type is not available on this platform
    MissingPlatformSupport{reason: &'static str, feature: &'static str},

    /// Inspecion of current process environment variable to serve `sd-listen` failed because of missing or malformed value
    EvnVarError{reason: &'static str, var:&'static str, fault:&'static str},

    /// Attempt to use [`crate::Listener::bind_multiple`] with empty slice of addresses
    MultiBindWithoutAddresses,

    /// There is some invalid value in [`crate::UserOptions`]
    InvalidUserOption { name : &'static str},
}

impl std::error::Error for BindError {

}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindError::MissingCompileTimeFeature { reason, feature } => write!(f, "tokio-listener: cannot {reason} because of {feature} was not enabled at compile time"),
            BindError::MissingPlatformSupport { reason, feature } => write!(f, "tokio-listener: cannot {reason} because of it is not a {feature}"),
            BindError::EvnVarError { var, fault, reason } => write!(f, "tokio-listener: cannot {reason} due to problem with environment variable {var}: {fault}"),
            BindError::MultiBindWithoutAddresses => write!(f, "tokio-listener: no addresses specified to bind to"),
            BindError::InvalidUserOption { name } => write!(f, "tokio-listener: invalid value for user option {name}"),
        }
    }
}

impl BindError {
    pub(crate) fn to_io<T> (self) -> Result<T, std::io::Error> {
        Err(std::io::Error::new(std::io::ErrorKind::Other, self))
    }
}


pub(crate) fn get_envvar(reason: &'static str, var: &'static str) -> Result<String, std::io::Error> {
    match std::env::var(var) {
        Ok(x) => Ok(x),
        Err(e) => match e {
            std::env::VarError::NotPresent => BindError::EvnVarError { reason, var, fault: "not present" }.to_io(),
            std::env::VarError::NotUnicode(..) =>BindError::EvnVarError { reason, var, fault: "not unicode" }.to_io(),
        }
    }
}

/// `tokio-listener`-specific accept errors, to be packed in [`std::io::Error::other`].
#[derive(Debug)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum AcceptError {
    /// In inetd mode we have already accepted and served one connection.
    /// 
    /// It is not possible to serve another connection, hence this error is used to bubble up from 
    /// accept loop and exit process.
    /// 
    /// `tokio-listener` takes care not to trigger this error prematurely, when the first connection is still being served.
    InetdPseudosocketAlreadyTaken,
}

impl std::error::Error for AcceptError {

}

impl std::fmt::Display for AcceptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcceptError::InetdPseudosocketAlreadyTaken => write!(f, "tokio-listener: Cannot serve a second connection in inetd mode"),
        }
    }
}
