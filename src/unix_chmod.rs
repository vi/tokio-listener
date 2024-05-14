use std::{fmt::Display, str::FromStr};

#[cfg_attr(docsrs_alt, doc(cfg(feature = "unix_path_tools")))]
#[cfg(feature = "unix_path_tools")]
/// Value of `--unix-listen-chmod` option which allows changing DAC file access mode for UNIX path socket
#[non_exhaustive]
#[cfg_attr(
    feature = "serde",
    derive(serde_with::DeserializeFromStr, serde_with::SerializeDisplay)
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnixChmodVariant {
    /// Set filesystem mode of the UNIX socket to `u+rw`, allowing access only to one uid
    Owner,
    /// Set filesystem mode of the UNIX socket to `ug+rw`, allowing access to owner uid and a group
    Group,
    /// Set filesystem mode of the UNIX socket to `a+rw`, allowing global access to the socket
    Everybody,
}

#[cfg(feature = "unix_path_tools")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "unix_path_tools")))]
impl Display for UnixChmodVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnixChmodVariant::Owner => "owner".fmt(f),
            UnixChmodVariant::Group => "group".fmt(f),
            UnixChmodVariant::Everybody => "everybody".fmt(f),
        }
    }
}

#[cfg(feature = "unix_path_tools")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "unix_path_tools")))]
impl FromStr for UnixChmodVariant {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("owner") {
            Ok(UnixChmodVariant::Owner)
        } else if s.eq_ignore_ascii_case("group") {
            Ok(UnixChmodVariant::Group)
        } else if s.eq_ignore_ascii_case("everybody") {
            Ok(UnixChmodVariant::Everybody)
        } else {
            Err("Unknown chmod variant. Expected `owner`, `group` or `everybody`.")
        }
    }
}
