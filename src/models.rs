use std::collections::BTreeMap;
use std::time::SystemTime;

use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::providers::ProviderId;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageSection {
    pub percentage: f64,
    pub resets_at: Option<SystemTime>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageData {
    pub session: UsageSection,
    pub weekly: UsageSection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_label: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppUsageData {
    providers: BTreeMap<ProviderId, UsageData>,
    pub accounts: crate::accounts::AccountUsages,
    pub account_order: Vec<crate::accounts::Account>,
}

impl AppUsageData {
    pub fn refresh_codex_alias(&mut self) {
        self.providers.remove(&ProviderId::Codex);
        if let Some(usage) = self
            .account_order
            .iter()
            .find(|account| account.visible)
            .and_then(|account| self.accounts.get(&account.id))
            .and_then(|entry| entry.usage.clone())
        {
            self.providers.insert(ProviderId::Codex, usage);
        }
    }

    pub fn all_usage(&self) -> impl Iterator<Item = &UsageData> {
        self.providers
            .iter()
            .filter(|(provider, _)| {
                **provider != ProviderId::Codex || self.account_order.is_empty()
            })
            .map(|(_, usage)| usage)
            .chain(
                self.account_order
                    .iter()
                    .filter(|a| a.visible)
                    .filter_map(|a| self.accounts.get(&a.id))
                    .filter(|entry| entry.error.is_none())
                    .filter_map(|entry| entry.usage.as_ref()),
            )
    }
    pub fn get(&self, provider: ProviderId) -> Option<&UsageData> {
        self.providers.get(&provider)
    }

    pub fn insert(&mut self, provider: ProviderId, usage: UsageData) -> Option<UsageData> {
        self.providers.insert(provider, usage)
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

impl FromIterator<(ProviderId, UsageData)> for AppUsageData {
    fn from_iter<T: IntoIterator<Item = (ProviderId, UsageData)>>(iter: T) -> Self {
        Self {
            providers: iter.into_iter().collect(),
            ..Default::default()
        }
    }
}

impl Serialize for AppUsageData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        for (provider, usage) in &self.providers {
            map.serialize_entry(provider.descriptor().cache_key, usage)?;
        }
        if !self.accounts.is_empty() || !self.account_order.is_empty() {
            map.serialize_entry("accounts", &self.accounts)?;
            map.serialize_entry("account_order", &self.account_order)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for AppUsageData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut values = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
        let accounts = values
            .remove("accounts")
            .map(serde_json::from_value)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .unwrap_or_default();
        let account_order = values
            .remove("account_order")
            .map(serde_json::from_value)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .unwrap_or_default();
        let mut result: Self = values
            .into_iter()
            .filter_map(|(key, usage)| {
                let usage: UsageData = serde_json::from_value(usage).ok()?;
                ProviderId::from_cache_key(&key)
                    .or_else(|| ProviderId::from_key(&key))
                    .map(|provider| (provider, usage))
            })
            .collect();
        result.accounts = accounts;
        result.account_order = account_order;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_cache_keeps_legacy_provider_keys() {
        let data: AppUsageData = [
            (ProviderId::Claude, UsageData::default()),
            (ProviderId::Codex, UsageData::default()),
            (
                ProviderId::OpenCode,
                UsageData {
                    weekly_label: Some("30d".into()),
                    ..Default::default()
                },
            ),
        ]
        .into_iter()
        .collect();

        let json = serde_json::to_value(&data).unwrap();
        assert!(json.get("claude_code").is_some());
        assert!(json.get("codex").is_some());
        assert_eq!(json["opencode"]["weekly_label"], "30d");
        assert!(json.get("claude").is_none());

        let decoded: AppUsageData = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn usage_cache_accepts_nulls_from_the_legacy_struct_format() {
        let decoded: AppUsageData = serde_json::from_str(
            r#"{
                "claude_code": null,
                "codex": {"session":{"percentage":42.0,"resets_at":null},"weekly":{"percentage":0.0,"resets_at":null}},
                "antigravity": null
                ,"opencode": null
            }"#,
        )
        .unwrap();

        assert!(decoded.get(ProviderId::Claude).is_none());
        assert_eq!(
            decoded.get(ProviderId::Codex).unwrap().session.percentage,
            42.0
        );
        assert!(decoded.get(ProviderId::Antigravity).is_none());
        assert!(decoded.get(ProviderId::OpenCode).is_none());
    }
}
