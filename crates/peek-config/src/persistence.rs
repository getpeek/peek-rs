/// Whether this build may write to `~/peek`. The rewrite runs read-only against the user's
/// real data until the document mutation milestone lands; every save path checks this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PersistenceMode {
    #[default]
    ReadOnly,
    ReadWrite,
}

impl PersistenceMode {
    #[must_use]
    pub fn can_write(self) -> bool {
        self == Self::ReadWrite
    }
}
