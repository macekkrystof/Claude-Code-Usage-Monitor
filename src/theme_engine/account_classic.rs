use super::*;
use crate::accounts::{Account, AccountUsage};

/// Expand only the bundled Classic scene. Custom themes retain their saved geometry.
pub fn account_classic(
    theme: &ThemeDocument,
    data: Option<&AppUsageData>,
    providers: ProviderSet,
) -> ThemeDocument {
    let mut result = theme.clone();
    let classic_compatible = theme.is_builtin_classic() || theme.id == "migrated-theme";
    if classic_compatible
        && theme.surfaces.first().is_some_and(|root| {
            root.children
                .iter()
                .any(|child| child.id.starts_with("column-"))
                || root.children.iter().any(|child| child.id == "no-accounts")
        })
    {
        result = ThemeDocument::starter();
        result.surfaces[0].placement = theme.surfaces[0].placement.clone();
        result.surfaces[0].render = theme.surfaces[0].render.clone();
    }
    if !classic_compatible || !providers.contains(ProviderId::Codex) {
        return result;
    }
    let Some(data) = data else {
        return result;
    };
    {
        let Some(root) = result.surfaces.first_mut() else {
            return result;
        };
        root.children.clear();
        let mut columns = Vec::new();
        for descriptor in PROVIDER_DESCRIPTORS {
            if !providers.contains(descriptor.id) {
                continue;
            }
            if descriptor.id == ProviderId::Codex {
                columns.extend(data.account_order.iter().filter(|a| a.visible).map(|a| {
                    (
                        format!("accounts.{}", a.id),
                        a.name.clone(),
                        format!("{{accounts.{}.color}}", a.id),
                    )
                }));
            } else {
                columns.push((
                    descriptor.key.into(),
                    descriptor.display_name.into(),
                    [
                        "#D58D6FFF",
                        "#4D9FFFFF",
                        "#47C59CFF",
                        "#BA93E9FF",
                        "#6AB8D5FF",
                    ][descriptor.id as usize]
                        .into(),
                ));
            }
        }
        root.width = ((columns.len().max(1) * 182 + 8) as f64).into();
        root.height = 46.0.into();
        for (index, (key, name, color)) in columns.iter().enumerate() {
            let mut column = SceneObject::object(format!("column-{key}"), name);
            column.x = (4.0 + index as f64 * 182.0).into();
            column.width = 178.0.into();
            column.height = 46.0.into();
            let title = if name.chars().count() > 23 {
                format!("{}…", name.chars().take(22).collect::<String>())
            } else {
                name.clone()
            };
            // Literal names are escaped, so user-supplied braces cannot become expressions.
            column.children.push(text(
                &format!("{key}-title"),
                &title.replace('{', "{{"),
                0.0,
                0.0,
                if key.starts_with("accounts.") {
                    88.0
                } else {
                    176.0
                },
                color,
                11.0,
            ));
            if key.starts_with("accounts.") {
                let mut reset = text(
                    &format!("{key}-reset-credits"),
                    &format!("{{{key}.reset_credits.suffix}}"),
                    90.0,
                    0.0,
                    88.0,
                    color,
                    8.0,
                );
                reset.render = Expression(format!("{key}.reset_credits.present"));
                if let SceneContent::Text { align, .. } = &mut reset.content {
                    *align = TextAlign::Right;
                }
                column.children.push(reset);
            }
            for (window, y, label) in [("session", 14.0, "5h"), ("weekly", 29.0, "7d")] {
                let label = if key == "cursor" {
                    if window == "session" {
                        "Auto".into()
                    } else {
                        "API".into()
                    }
                } else if window == "weekly" {
                    format!("{{{key}.weekly.label}}")
                } else {
                    label.into()
                };
                column.children.push(text(
                    &format!("{key}-{window}-label"),
                    &label,
                    0.0,
                    y,
                    25.0,
                    color,
                    10.0,
                ));
                let mut bar = SceneObject::object(format!("{key}-{window}-bar"), "Usage");
                bar.x = 27.0.into();
                bar.y = (y + 3.0).into();
                bar.width = 58.0.into();
                bar.height = 8.0.into();
                bar.visibility = Expression(if key.starts_with("accounts.") {
                    format!("if({key}.has_error, 35, 100)")
                } else {
                    "100".into()
                });
                bar.content = SceneContent::Progress {
                    value: Expression(format!("{key}.{window}.percentage")),
                    direction: ProgressDirection::default(),
                    fill: Paint::new(color),
                    track: Paint::new("#80808040"),
                    corner_radius: 2.0.into(),
                    segments: 5,
                    segments_expression: None,
                    segment_gap: 2.0.into(),
                };
                column.children.push(bar);
                column.children.push(text(
                    &format!("{key}-{window}-value"),
                    &format!("{{{key}.{window}:usage_line}}"),
                    90.0,
                    y,
                    88.0,
                    color,
                    10.0,
                ));
            }
            let mut children = std::mem::take(&mut column.children);
            for child in &mut children {
                child.parent = Some(column.id.clone());
            }
            root.children.push(column);
            root.children.extend(children);
        }
        if columns.is_empty() {
            root.children.push(text(
                "no-accounts",
                "Codex — --",
                4.0,
                14.0,
                176.0,
                "#808080FF",
                11.0,
            ));
        }
    }
    expand_codex_tray_surfaces(&mut result, data);
    result.prepare_runtime();
    result
}

fn expand_codex_tray_surfaces(theme: &mut ThemeDocument, data: &AppUsageData) {
    let Some(template_index) = theme
        .surfaces
        .iter()
        .position(|surface| surface.id == "codex-tray-icon")
    else {
        return;
    };
    let template = theme.surfaces[template_index].clone();
    let replacements = data
        .account_order
        .iter()
        .map(|account| account_tray_surface(&template, account))
        .collect::<Vec<_>>();
    theme
        .surfaces
        .splice(template_index..=template_index, replacements);
}

fn account_tray_surface(template: &SceneObject, account: &Account) -> SceneObject {
    let key = format!("accounts.{}", account.id);
    let root_id = format!("codex-account-tray-{}", account.id);
    let mut surface = template.clone();
    surface.id = root_id.clone();
    surface.name = format!("Codex — {}", account.name);
    surface.render = Expression(format!("{key}.enabled"));
    surface.background = LayerBackground::Colour {
        colour: Paint::new(&format!("{{{key}.color}}")),
    };

    let original_ids = surface
        .children
        .iter()
        .map(|child| child.id.clone())
        .collect::<Vec<_>>();
    for child in &mut surface.children {
        let original_id = child.id.clone();
        child.id = format!("{original_id}-{}", account.id);
        child.render = replace_codex_reference(&child.render, &key);
        child.visibility = replace_codex_reference(&child.visibility, &key);
        child.x = replace_codex_reference(&child.x, &key);
        child.y = replace_codex_reference(&child.y, &key);
        child.width = replace_codex_reference(&child.width, &key);
        child.height = replace_codex_reference(&child.height, &key);
        child.rotation = replace_codex_reference(&child.rotation, &key);
        child.corner_radius = replace_codex_reference(&child.corner_radius, &key);
        child.gap = replace_codex_reference(&child.gap, &key);
        match &mut child.content {
            SceneContent::Text {
                template,
                font_size,
                contrast,
                color,
                ..
            } => {
                *template = replace_codex_text(template, &key);
                *font_size = replace_codex_reference(font_size, &key);
                *contrast = replace_codex_reference(contrast, &key);
                color.color = replace_codex_text(&color.color, &key);
                color.opacity = replace_codex_reference(&color.opacity, &key);
            }
            SceneContent::Progress {
                value,
                fill,
                track,
                corner_radius,
                segment_gap,
                segments_expression,
                ..
            } => {
                *value = replace_codex_reference(value, &key);
                fill.color = replace_codex_text(&fill.color, &key);
                fill.opacity = replace_codex_reference(&fill.opacity, &key);
                track.color = replace_codex_text(&track.color, &key);
                track.opacity = replace_codex_reference(&track.opacity, &key);
                *corner_radius = replace_codex_reference(corner_radius, &key);
                *segment_gap = replace_codex_reference(segment_gap, &key);
                if let Some(expression) = segments_expression {
                    *expression = replace_codex_reference(expression, &key);
                }
            }
            SceneContent::None => {}
        }
        if original_id == "codex-tray-low-fill" {
            child.background = LayerBackground::Colour {
                colour: Paint::new(&format!("{{{key}.color}}")),
            };
        }
    }
    for child in &mut surface.children {
        child.parent = child.parent.as_ref().and_then(|parent| {
            if parent == &template.id {
                Some(root_id.clone())
            } else {
                original_ids
                    .iter()
                    .position(|id| id == parent)
                    .map(|index| format!("{}-{}", original_ids[index], account.id))
            }
        });
    }
    surface
}

fn replace_codex_reference(expression: &Expression, key: &str) -> Expression {
    Expression(expression.0.replace("codex.", &format!("{key}.")))
}

fn replace_codex_text(value: &str, key: &str) -> String {
    value.replace("codex.", &format!("{key}."))
}

pub fn codex_account_tray_tooltip(
    account: &Account,
    usage: Option<&AccountUsage>,
    language: LanguageId,
) -> String {
    let mut tooltip = format!("Codex — {}", account.name);
    if let Some(label) = usage
        .and_then(|entry| entry.usage.as_ref())
        .and_then(|usage| usage.reset_credits_available)
        .map(|count| format!("{count} {}", language.text("resets available")))
    {
        tooltip.push_str(" · ");
        tooltip.push_str(&label);
    }
    tooltip
}

fn text(
    id: &str,
    template: &str,
    x: f64,
    y: f64,
    width: f64,
    color: &str,
    size: f64,
) -> SceneObject {
    let mut object = SceneObject::object(id, id);
    object.x = x.into();
    object.y = y.into();
    object.width = width.into();
    object.height = 14.0.into();
    object.content = SceneContent::Text {
        template: template.into(),
        font_family: "Segoe UI".into(),
        font_size: size.into(),
        weight: FontWeight::Medium,
        rendering: FontRendering::default(),
        contrast: 1.0.into(),
        align: TextAlign::default(),
        color: Paint::new(color),
    };
    object
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{Account, AccountUsage};

    fn fixture() -> (AppUsageData, ProviderSet) {
        let mut first = Account::new(0);
        first.name = "Osobní".into();
        let mut second = Account::new(1);
        second.name = "Pracovní".into();
        let mut data = AppUsageData::default();
        crate::accounts::reconcile(&mut data, &[first.clone(), second.clone()]);
        for (account, percent) in [(first, 23.0), (second, 78.0)] {
            let mut usage = crate::models::UsageData::default();
            usage.session.percentage = percent;
            usage.weekly.percentage = percent / 2.0;
            usage.session.resets_at =
                Some(std::time::SystemTime::now() + std::time::Duration::from_secs(9000));
            usage.weekly.resets_at =
                Some(std::time::SystemTime::now() + std::time::Duration::from_secs(180000));
            data.accounts.insert(
                account.id,
                AccountUsage {
                    usage: Some(usage),
                    updated_unix: 1,
                    error: None,
                },
            );
        }
        data.refresh_codex_alias();
        (data, ProviderSet::from_enabled([ProviderId::Codex]))
    }

    #[test]
    fn classic_renders_two_independent_columns_at_multiple_dpi_scales() {
        let (data, providers) = fixture();
        let theme = account_classic(&ThemeDocument::starter(), Some(&data), providers);
        assert!(theme.validate().is_empty(), "{:?}", theme.validate());
        assert_eq!(
            theme.surfaces[0]
                .children
                .iter()
                .filter(|c| c.id.starts_with("column-"))
                .count(),
            2
        );
        assert_eq!(
            theme.surfaces.len(),
            ThemeDocument::starter().surfaces.len() + 1
        );
        let runtime = ThemeRuntime::from_providers(providers);
        let context = DataContext::from_usage_with_runtime(Some(&data), &theme.canvas, runtime);
        assert_eq!(context.get("accounts.count"), Some(2.0));
        assert_eq!(context.get("providers.count"), Some(1.0));
        assert!(format_template(
            &format!(
                "{{accounts.{}.session:usage_line}}",
                data.account_order[0].id
            ),
            &context
        )
        .starts_with("23%"));
        for scale in [1.0, 1.5, 2.0] {
            let rendered =
                render_theme_surface_with_runtime_at_scale(&theme, 0, Some(&data), runtime, scale);
            assert!(rendered.warnings.is_empty(), "{:?}", rendered.warnings);
            assert_eq!(rendered.width, (372.0 * scale) as u32);
            assert_eq!(rendered.height, (46.0 * scale) as u32);
            // Save an opaque dark-background preview for visual inspection, using actual renderer output.
            let bytes: Vec<u8> = rendered
                .pixels
                .iter()
                .flat_map(|pixel| {
                    let alpha = (pixel >> 24) & 255;
                    [
                        ((pixel >> 16) & 255) as u8,
                        ((pixel >> 8) & 255) as u8,
                        (pixel & 255) as u8,
                    ]
                    .map(|value| (value as u32 + 28 * (255 - alpha) / 255).min(255) as u8)
                })
                .collect();
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "target/codex-accounts-{}.png",
                (scale * 100.0) as u32
            ));
            image::save_buffer(
                path,
                &bytes,
                rendered.width,
                rendered.height,
                image::ColorType::Rgb8,
            )
            .unwrap();
        }
    }

    #[test]
    fn classic_creates_ordered_account_tray_icons_and_hides_hidden_accounts() {
        let (mut data, providers) = fixture();
        let first_id = data.account_order[0].id.clone();
        let second_id = data.account_order[1].id.clone();
        data.account_order[1].visible = false;

        let theme = account_classic(&ThemeDocument::starter(), Some(&data), providers);
        let tray = theme
            .surfaces
            .iter()
            .filter(|surface| surface.id.starts_with("codex-account-tray-"))
            .collect::<Vec<_>>();
        assert_eq!(
            tray.iter()
                .map(|surface| surface.id.to_string())
                .collect::<Vec<_>>(),
            vec![
                format!("codex-account-tray-{first_id}"),
                format!("codex-account-tray-{second_id}"),
            ]
        );
        assert!(surface_should_render(
            &theme,
            theme
                .surfaces
                .iter()
                .position(|surface| surface.id == format!("codex-account-tray-{first_id}"))
                .unwrap(),
            Some(&data),
            ThemeRuntime::from_providers(providers),
        ));
        assert!(!surface_should_render(
            &theme,
            theme
                .surfaces
                .iter()
                .position(|surface| surface.id == format!("codex-account-tray-{second_id}"))
                .unwrap(),
            Some(&data),
            ThemeRuntime::from_providers(providers),
        ));
        assert!(matches!(
            &tray[0].background,
            LayerBackground::Colour { .. }
        ));
        assert!(tray[0]
            .children
            .iter()
            .any(|child| child.render.0.contains(first_id.as_str())));
    }

    #[test]
    fn classic_removes_account_tray_icons_when_accounts_are_removed() {
        let (mut data, providers) = fixture();
        let removed_id = data.account_order.pop().unwrap().id;
        data.accounts.remove(&removed_id);
        let theme = account_classic(&ThemeDocument::starter(), Some(&data), providers);
        assert_eq!(
            theme
                .surfaces
                .iter()
                .filter(|surface| surface.id.starts_with("codex-account-tray-"))
                .count(),
            1
        );
        assert!(!theme
            .surfaces
            .iter()
            .any(|surface| surface.id == "codex-tray-icon"));
    }

    #[test]
    fn account_tray_tooltip_localizes_available_reset_count() {
        let (data, _) = fixture();
        let account = &data.account_order[0];
        let usage = data.accounts.get(&account.id).unwrap();
        let mut usage = usage.clone();
        usage.usage.as_mut().unwrap().reset_credits_available = Some(3);
        assert_eq!(
            codex_account_tray_tooltip(account, Some(&usage), LanguageId::Czech),
            format!("Codex — {} · 3 resetů k dispozici", account.name)
        );
        usage.usage.as_mut().unwrap().reset_credits_available = None;
        assert_eq!(
            codex_account_tray_tooltip(account, Some(&usage), LanguageId::Czech),
            format!("Codex — {}", account.name)
        );
    }

    #[test]
    fn account_error_preserves_geometry_and_custom_themes_survive_removal() {
        let (mut data, providers) = fixture();
        let theme = account_classic(&ThemeDocument::starter(), Some(&data), providers);
        let first = data.account_order[0].id.clone();
        data.accounts.get_mut(&first).unwrap().error = Some("Sign in again".into());
        let changed = account_classic(&theme, Some(&data), providers);
        assert_eq!(changed.canvas.width, theme.canvas.width);
        let context = DataContext::from_usage_with_runtime(
            Some(&data),
            &theme.canvas,
            ThemeRuntime::from_providers(providers),
        );
        assert_eq!(
            format_template(
                &format!("{{accounts.{first}.session:usage_line}}"),
                &context
            ),
            "!"
        );
        let mut custom = theme.clone();
        custom.id = "my-accounts".into();
        assert!(custom.validate().is_empty());
        let encoded = serde_json::to_string(&custom).unwrap();
        let decoded: ThemeDocument = serde_json::from_str(&encoded).unwrap();
        assert!(decoded.validate().is_empty());
        let no_accounts = AppUsageData::default();
        let unchanged = account_classic(&custom, Some(&no_accounts), providers);
        assert_eq!(unchanged.canvas.width, custom.canvas.width);
        assert!(render_theme_surface_with_runtime(
            &unchanged,
            0,
            Some(&no_accounts),
            ThemeRuntime::from_providers(providers)
        )
        .warnings
        .is_empty());
        data.account_order[0].visible = false;
        let hidden = account_classic(&theme, Some(&data), providers);
        assert_eq!(hidden.canvas.width, 190);
        let restored = account_classic(&hidden, Some(&data), ProviderSet::default());
        assert_eq!(
            restored.surfaces[0].children.len(),
            ThemeDocument::starter().surfaces[0].children.len()
        );
    }

    #[test]
    fn migrated_classic_theme_expands_accounts_and_keeps_legacy_placement() {
        let (data, providers) = fixture();
        let theme = ThemeDocument::migrated_from_legacy(Some((1, -37)), true);
        let expanded = account_classic(&theme, Some(&data), providers);
        assert_eq!(
            expanded
                .surfaces
                .first()
                .map(|surface| surface.placement.reference.display),
            Some(1usize)
        );
        assert_eq!(
            expanded
                .surfaces
                .first()
                .map(|surface| surface.placement.offset_x),
            Some(-37)
        );
        assert_eq!(
            expanded
                .surfaces
                .first()
                .into_iter()
                .flat_map(|surface| surface.children.iter())
                .filter(|child| child.id.starts_with("column-"))
                .count(),
            2
        );
        assert!(render_theme_surface_with_runtime(
            &expanded,
            0,
            Some(&data),
            ThemeRuntime::from_providers(providers)
        )
        .warnings
        .is_empty());
    }
}
