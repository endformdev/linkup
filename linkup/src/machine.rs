use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable identity for a Linkup installation on a machine.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct MachineId(Uuid);

impl MachineId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for MachineId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for MachineId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_unique() {
        let id = MachineId::generate();

        assert_ne!(id, MachineId::generate());
        assert_eq!(id.0.get_version_num(), 4);
    }

    #[test]
    fn display_value_can_be_parsed() {
        let id = MachineId::generate();

        assert_eq!(id.to_string().parse::<MachineId>().unwrap(), id);
    }

    #[test]
    fn serializes_as_a_uuid_string() {
        let id = MachineId::generate();
        let serialized = serde_json::to_string(&id).unwrap();

        assert_eq!(serialized, format!("\"{id}\""));
        assert_eq!(serde_json::from_str::<MachineId>(&serialized).unwrap(), id);
    }
}
