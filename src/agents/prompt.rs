//! Interactive multi-select prompt for agent selection.

use super::{Agent, DetectionLevel};

pub fn prompt_agents<'a>(
    detection: &[(&'a Agent, DetectionLevel, String)],
) -> Result<Vec<&'a Agent>, Box<dyn std::error::Error>> {
    use dialoguer::MultiSelect;
    use dialoguer::theme::ColorfulTheme;

    let labels: Vec<String> = detection
        .iter()
        .map(|(a, lvl, reason)| match lvl {
            DetectionLevel::Active | DetectionLevel::Installed => {
                format!("{:<24} (detected: {})", a.display_name, reason)
            }
            DetectionLevel::Unknown => format!("{:<24} (not detected)", a.display_name),
        })
        .collect();

    let defaults: Vec<bool> = detection
        .iter()
        .map(|(_, lvl, _)| matches!(lvl, DetectionLevel::Active | DetectionLevel::Installed))
        .collect();

    let selected_indices = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Which agents should illu configure?")
        .items(&labels)
        .defaults(&defaults)
        .interact()?;

    Ok(selected_indices
        .into_iter()
        .map(|i| detection[i].0)
        .collect())
}

#[must_use]
pub fn has_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}
