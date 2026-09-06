//! Account metadata is public state; credentials live only in the encrypted vault.
use crate::models::UsageData;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type AccountId = String;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountSource {
    LocalCli,
    OAuth,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub color: String,
    pub visible: bool,
    pub source: AccountSource,
    #[serde(default)]
    pub identity: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub workspace: String,
}

pub fn legacy_accounts() -> Vec<Account> {
    vec![Account {
        id: "codex_local".into(),
        name: "Codex – local CLI".into(),
        color: "#4D9FFFFF".into(),
        visible: true,
        source: AccountSource::LocalCli,
        identity: String::new(),
        subject: String::new(),
        workspace: String::new(),
    }]
}

impl Account {
    pub fn new(index: usize) -> Self {
        use rand::RngCore;
        let mut bytes = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self {
            id: format!(
                "codex_{}",
                bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
            ),
            name: format!("Codex {}", index + 1),
            color: ["#4D9FFFFF", "#B18AFFFF", "#35C9A0FF", "#F0A44BFF"][index % 4].into(),
            visible: true,
            source: AccountSource::OAuth,
            identity: String::new(),
            subject: String::new(),
            workspace: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountUsage {
    pub usage: Option<UsageData>,
    pub updated_unix: u64,
    pub error: Option<String>,
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Retain only configured accounts, and seed the old single-provider cache once.
pub fn reconcile(data: &mut crate::models::AppUsageData, accounts: &[Account]) {
    data.accounts
        .retain(|id, _| accounts.iter().any(|account| &account.id == id));
    let legacy = data.get(crate::providers::ProviderId::Codex).cloned();
    for account in accounts {
        data.accounts
            .entry(account.id.clone())
            .or_insert_with(|| AccountUsage {
                usage: if account.source == AccountSource::LocalCli {
                    legacy.clone()
                } else {
                    None
                },
                ..Default::default()
            });
    }
    data.account_order = accounts.to_vec();
    data.refresh_codex_alias();
}

pub fn identity_match(accounts: &[Account], incoming: &Account) -> Option<usize> {
    accounts.iter().position(|account| {
        account.source == AccountSource::OAuth
            && account.subject == incoming.subject
            && account.workspace == incoming.workspace
    })
}

pub type AccountUsages = BTreeMap<AccountId, AccountUsage>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::AppUsageData, providers::ProviderId};

    #[test]
    fn migration_and_removal_do_not_cross_account_data() {
        let mut data = AppUsageData::default();
        let mut usage = UsageData::default();
        usage.session.percentage = 41.0;
        data.insert(ProviderId::Codex, usage);
        let mut accounts = legacy_accounts();
        accounts.push(Account::new(0));
        reconcile(&mut data, &accounts);
        assert_eq!(
            data.accounts["codex_local"]
                .usage
                .as_ref()
                .unwrap()
                .session
                .percentage,
            41.0
        );
        assert!(data.accounts[&accounts[1].id].usage.is_none());
        let encoded = serde_json::to_string(&data).unwrap();
        let decoded: AppUsageData = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, data);
        accounts.remove(0);
        reconcile(&mut data, &accounts);
        assert!(!data.accounts.contains_key("codex_local"));
        assert!(data.get(ProviderId::Codex).is_none());
    }

    #[test]
    fn deduplication_uses_subject_and_workspace_not_name_or_email() {
        let mut first = Account::new(0);
        first.subject = "user1".into();
        first.workspace = "team1".into();
        let mut second = first.clone();
        second.id = "codex_other".into();
        second.name = "Personal".into();
        assert_eq!(identity_match(&[first.clone()], &second), Some(0));
        second.workspace = "team2".into();
        assert_eq!(identity_match(&[first], &second), None);
    }

    #[test]
    fn alias_follows_first_visible_account_without_falling_through_to_another() {
        let mut accounts = vec![Account::new(0), Account::new(1)];
        let mut data = AppUsageData::default();
        reconcile(&mut data, &accounts);
        data.accounts.get_mut(&accounts[1].id).unwrap().usage = Some(UsageData::default());
        data.refresh_codex_alias();
        assert!(data.get(ProviderId::Codex).is_none());
        accounts[0].visible = false;
        reconcile(&mut data, &accounts);
        assert!(data.get(ProviderId::Codex).is_some());
        accounts[1].visible = false;
        reconcile(&mut data, &accounts);
        assert!(data.get(ProviderId::Codex).is_none());
        assert_eq!(data.all_usage().count(), 0);
    }
}
