use std::path::PathBuf;

use chrono::Utc;
use log4rs::append::console::ConsoleAppender;
use log4rs::append::file::FileAppender;
use log4rs::encode::pattern::PatternEncoder;
use log::LevelFilter;
use regex::Regex;

use crate::constants::LOGS_DIR;

pub fn initialize_logs(config_dir: &PathBuf, silent: bool) {
    let log_filename = format!("{:}.log", Regex::new(r"[:.+ ]").unwrap().replace_all(Utc::now().to_rfc3339().as_str(), "-"));

    let mut config_builder = log4rs::config::runtime::Config::builder()
        .appender(
            log4rs::config::Appender::builder().build("logfile", Box::new(
                FileAppender::builder()
                    .encoder(Box::new(PatternEncoder::new("{d(%Y-%m-%dT%H:%M:%S)} | {({l}):5.5} | {m}{n}")))
                    .build(config_dir.join(LOGS_DIR).join(log_filename)).unwrap()),
            )
        );

    if !silent {
        config_builder = config_builder.appender(
            log4rs::config::Appender::builder().build("console", Box::new(
                ConsoleAppender::builder()
                    .encoder(Box::new(PatternEncoder::new("{d(%Y-%m-%dT%H:%M:%S)} | {({l}):5.5} | {m}{n}")))
                    .build(),
            ),
            )
        );
    }

    let mut log_builder = log4rs::config::Root::builder()
        .appender("logfile");

    if !silent {
        log_builder = log_builder.appender("console");
    }

    log4rs::init_config(config_builder.build(log_builder.build(LevelFilter::Info)).unwrap()).unwrap();
    log::info!("Logs initialized");
}