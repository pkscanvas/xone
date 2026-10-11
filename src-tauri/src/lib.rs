mod ai;
mod behavior;
mod commands;
mod content;
mod db;
mod dedup;
mod defaults;
mod drain;
mod feeds;
mod scheduler;
mod security;
mod tldr;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            security::private_directory(&app_data_dir)?;
            let db_path = app_data_dir.join("zonereader.sqlite3");
            let is_new_install = std::fs::metadata(&db_path)
                .map(|metadata| metadata.len() == 0)
                .unwrap_or(true);
            let db = db::Db::new(&db_path)?;
            if is_new_install {
                db.seed_default_zones()?;
            }
            if let Err(err) = commands::cleanup_old_articles_if_due(&db) {
                eprintln!("Could not clean up old articles at startup: {err}");
            }
            app.manage(db);
            app.manage(behavior::BehaviorLearningControl::new());
            app.manage(scheduler::SchedulerControl::new());
            app.manage(drain::DrainControl::new());
            behavior::start(app.handle().clone());
            scheduler::start(app.handle().clone());
            app.manage(tldr::TldrControl::default());
            tldr::start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            tldr::get_tldr,
            tldr::regenerate_tldr,
            commands::list_feeds,
            commands::add_feed,
            commands::refresh_feed,
            commands::remove_feed,
            commands::get_all_articles,
            commands::get_smart_feed_counts,
            commands::get_today_articles,
            commands::get_unread_articles,
            commands::list_zones,
            commands::create_zone,
            commands::update_zone,
            commands::delete_zone,
            commands::reorder_zones,
            commands::assign_feed,
            commands::unassign_feed,
            commands::get_zone_feeds,
            commands::get_zone_articles,
            commands::mark_read,
            commands::retry_zone_behavior,
            commands::reset_zone_behavior,
            commands::get_article_cleanup_count,
            commands::cleanup_old_articles,
            commands::get_app_settings,
            commands::update_app_settings,
            commands::get_ai_settings,
            commands::update_ai_settings,
            commands::get_ai_status,
            commands::check_ai_status,
            commands::fetch_ai_models,
            commands::assist_filter_brief,
            commands::recommend_sources,
            commands::classify_zone,
            commands::open_in_browser
        ])
        .run(tauri::generate_context!())
        .expect("error while running Xone");
}
