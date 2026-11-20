//! Tauri commands for the "Permission rules" Settings page and the peek's
//! "Make a rule…" link. Every change emits `rules:changed` so open views refresh.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::{logic, Answer, DryRun, Rule};
use crate::AppState;

/// A rule made from a request, waiting for the Settings page to pick it up.
static DRAFT: Mutex<Option<Rule>> = Mutex::new(None);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RulesView {
    pub rules: Vec<Rule>,
    /// Shown above the list when rules.json could not be read.
    pub error: Option<String>,
    pub answers: usize,
    pub recorder_connected: bool,
    pub max_auto_days: u64,
}

pub fn changed(app: &AppHandle) {
    let _ = app.emit("rules:changed", ());
}

#[tauri::command]
pub async fn rules_list() -> RulesView {
    tauri::async_runtime::spawn_blocking(|| {
        let (rules, error) = super::list();
        RulesView { rules, error, answers: super::answers().len(), recorder_connected: false, max_auto_days: logic::MAX_AUTO_DAYS }
    })
    .await
    .unwrap_or(RulesView { rules: vec![], error: Some("Atoll could not load your rules.".into()), answers: 0, recorder_connected: false, max_auto_days: logic::MAX_AUTO_DAYS })
}

#[tauri::command]
pub fn rules_save(app: AppHandle, rule: Rule) -> Result<Rule, String> {
    let saved = super::save(rule)?;
    changed(&app);
    Ok(saved)
}

#[tauri::command]
pub fn rules_delete(app: AppHandle, id: String) -> Result<(), String> {
    super::delete(&id)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn rules_disable(app: AppHandle, id: String) -> Result<(), String> {
    super::disable(&id)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn rules_dry_run(rule: Rule) -> Result<DryRun, String> {
    tauri::async_runtime::spawn_blocking(move || super::dry_run(&rule)).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rules_log(limit: Option<usize>) -> Vec<Answer> {
    let limit = limit.unwrap_or(50).min(500);
    tauri::async_runtime::spawn_blocking(move || super::decision_log(limit)).await.unwrap_or_default()
}

/// The peek's "Make a rule…": a draft from the waiting request, then the Settings page.
#[tauri::command]
pub fn rules_from_request(app: AppHandle, id: u64) -> Result<(), String> {
    let (facts, project) = app.state::<AppState>().permissions.lock().unwrap().rule_facts(id).ok_or("That request is no longer waiting.")?;
    if facts.question {
        return Err("Rules can't answer Claude's questions.".into());
    }
    *DRAFT.lock().unwrap() = Some(logic::draft(&facts, &project, crate::sessions::now_ms()));
    crate::windows::open_dashboard(&app);
    let _ = app.emit("settings:goto", "rules");
    Ok(())
}

/// The waiting draft, once (the Settings page asks when it opens and on `settings:goto`).
#[tauri::command]
pub fn rules_take_draft() -> Option<Rule> {
    DRAFT.lock().unwrap().take()
}

/// Is a draft waiting? (The Settings window asks when it opens, without taking it.)
#[tauri::command]
pub fn rules_draft_waiting() -> bool {
    DRAFT.lock().unwrap().is_some()
}
