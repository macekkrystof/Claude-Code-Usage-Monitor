use super::*;
use crate::accounts::{Account, AccountSource};

pub(super) enum AccountChange {
    Saved(Account),
    Removed(String),
}

impl StudioApp {
    pub(super) fn update_accounts(&mut self) {
        if let Some(result) =
            self.account_login
                .as_ref()
                .and_then(|task| match task.receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        Some(Err("Sign-in failed. Try again".into()))
                    }
                    Err(_) => None,
                })
        {
            self.account_login = None;
            match result {
                Err(error) => self.account_error = Some(error),
                Ok(mut login) => {
                    if let Some(index) = crate::accounts::identity_match(
                        &self.settings.codex_accounts,
                        &login.account,
                    ) {
                        let existing = &self.settings.codex_accounts[index];
                        login.account.id = existing.id.clone();
                        login.account.name = existing.name.clone();
                        login.account.color = existing.color.clone();
                        login.account.visible = existing.visible;
                    }
                    let (sender, receiver) = std::sync::mpsc::channel();
                    self.account_job = Some(receiver);
                    std::thread::spawn(move || {
                        let result = crate::codex_oauth::store(&login.account.id, &login.tokens)
                            .map(|_| AccountChange::Saved(login.account));
                        let _ = sender.send(result);
                    });
                }
            }
        }
        if let Some(result) = self
            .account_job
            .as_ref()
            .and_then(|job| match job.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Unable to update account".into()))
                }
                Err(_) => None,
            })
        {
            self.account_job = None;
            match result {
                Err(error) => self.account_error = Some(error),
                Ok(change) => {
                    match change {
                        AccountChange::Saved(account) => {
                            if let Some(existing) = self
                                .settings
                                .codex_accounts
                                .iter_mut()
                                .find(|a| a.id == account.id)
                            {
                                *existing = account;
                            } else {
                                self.settings.codex_accounts.push(account);
                            }
                            self.settings
                                .set_provider_enabled(crate::providers::ProviderId::Codex, true);
                        }
                        AccountChange::Removed(id) => {
                            self.settings.codex_accounts.retain(|a| a.id != id)
                        }
                    }
                    self.account_error = None;
                    self.save_settings();
                    self.post_owner(WM_APP_REFRESH_NOW);
                }
            }
        }
    }

    pub(super) fn accounts_section(&mut self, ui: &mut egui::Ui) -> bool {
        let language = self.language();
        let busy = self.account_login.is_some() || self.account_job.is_some();
        let mut changed = false;
        let mut reconnect = None;
        let mut reorder = None;
        section(ui, language.text("Codex accounts"), |ui| {
            ui.label(language.text("Each account has its own name, colour and usage limits."));
            if let Some(error) = &self.account_error {
                ui.colored_label(
                    crate::ui::theme::danger(),
                    account_error_text(language, error),
                );
            }
            if self.account_login.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(language.text("Finish signing in in your browser"));
                    if ui.button(language.text("Cancel")).clicked() {
                        self.account_login = None;
                    }
                });
            } else if self.account_job.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(language.text("Saving account…"));
                });
            }
            let count = self.settings.codex_accounts.len();
            for (index, account) in self.settings.codex_accounts.iter_mut().enumerate() {
                setting_separator(ui);
                ui.push_id(account.id.clone(), |ui| {
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            changed |= ui
                                .checkbox(&mut account.visible, language.text("Show in widget"))
                                .changed();
                            changed |= ui
                                .add(singleline_text_edit(&mut account.name).desired_width(160.0))
                                .changed();
                            let previous = account.color.clone();
                            crate::ui::components::color_picker::color_string_field(
                                ui,
                                &mut account.color,
                                115.0,
                            );
                            changed |= previous != account.color;
                            if ui
                                .add_enabled(index > 0, egui::Button::new("↑"))
                                .on_hover_text(language.text("Move up"))
                                .clicked()
                            {
                                reorder = Some((index, index - 1));
                            }
                            if ui
                                .add_enabled(index + 1 < count, egui::Button::new("↓"))
                                .on_hover_text(language.text("Move down"))
                                .clicked()
                            {
                                reorder = Some((index, index + 1));
                            }
                            if account.source == AccountSource::OAuth
                                && ui.button(language.text("Sign in again")).clicked()
                            {
                                reconnect = Some(account.clone());
                            }
                            if ui.button(language.text("Remove")).clicked() {
                                self.account_delete = Some(account.id.clone());
                            }
                        });
                    });
                    if account.source == AccountSource::LocalCli {
                        ui.label(
                            egui::RichText::new(
                                language.text("Uses your existing local Codex CLI login"),
                            )
                            .color(muted()),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} · {}",
                                account.identity, account.workspace
                            ))
                            .color(muted()),
                        );
                    }
                    let entry = self
                        .usage
                        .as_ref()
                        .and_then(|data| data.accounts.get(&account.id));
                    ui.horizontal_wrapped(|ui| {
                        if let Some(error) = entry.and_then(|entry| entry.error.as_deref()) {
                            ui.colored_label(
                                crate::ui::theme::danger(),
                                account_error_text(language, error),
                            );
                        } else {
                            ui.label(language.text(
                                if entry.is_some_and(|entry| entry.usage.is_some()) {
                                    "Connected"
                                } else {
                                    "Waiting for usage"
                                },
                            ));
                        }
                        if let Some(entry) = entry {
                            if let Some(usage) = &entry.usage {
                                ui.label(format!(
                                    "5h: {:.0}% · 7d: {:.0}%",
                                    usage.session.percentage, usage.weekly.percentage
                                ));
                            }
                            if entry.updated_unix > 0 {
                                ui.label(format!(
                                    "{}: {} s",
                                    language.text("Last update"),
                                    crate::accounts::now_unix().saturating_sub(entry.updated_unix)
                                ));
                            }
                        }
                    });
                    if self.account_delete.as_deref() == Some(&account.id) {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(language.text("Remove this account from the monitor?"));
                            if ui
                                .add_enabled(
                                    !busy,
                                    egui::Button::new(language.text("Remove account")),
                                )
                                .clicked()
                            {
                                let id = account.id.clone();
                                let oauth = account.source == AccountSource::OAuth;
                                let (sender, receiver) = std::sync::mpsc::channel();
                                self.account_job = Some(receiver);
                                self.account_delete = None;
                                std::thread::spawn(move || {
                                    let result = if oauth {
                                        crate::codex_oauth::remove(&id)
                                    } else {
                                        Ok(())
                                    };
                                    let _ = sender.send(result.map(|_| AccountChange::Removed(id)));
                                });
                            }
                            if ui.button(language.text("Cancel")).clicked() {
                                self.account_delete = None;
                            }
                        });
                    }
                });
            }
            ui.add_space(8.0);
            if ui
                .add_enabled(!busy, egui::Button::new(language.text("Add Codex account")))
                .clicked()
            {
                let index = self.settings.codex_accounts.len();
                reconnect = Some(Account::new(index));
            }
        });
        if let Some((from, to)) = reorder {
            self.settings.codex_accounts.swap(from, to);
            changed = true;
        }
        if let Some(account) = reconnect {
            self.account_error = None;
            self.account_login = Some(crate::codex_oauth::start(account));
        }
        changed
    }
}

fn account_error_text(language: LanguageId, error: &str) -> &'static str {
    match error {
        "Sign in again" => language.text("Sign in again"),
        "Unable to refresh usage" => language.text("Unable to refresh usage"),
        "Sign-in ports are busy. Finish another sign-in and try again" => {
            language.text("Sign-in ports are busy. Finish another sign-in and try again")
        }
        "Sign-in timed out. Try again" => language.text("Sign-in timed out. Try again"),
        "A different account was selected. Use Add account instead" => {
            language.text("A different account was selected. Use Add account instead")
        }
        "Sign-in was declined" => language.text("Sign-in was declined"),
        _ => language.text("Unable to update account. Try signing in again"),
    }
}
