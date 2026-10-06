use std::io::BufRead;
use std::sync::Arc;

use pos_error::AppError;
use xc_auth::application::account::service::AccountService;
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xchange::bootstrap::Application;
use xchange::config::Settings;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args: Vec<String> = std::env::args().collect();

    match args.as_slice() {
        [_] => {
            let settings = Settings::load()?;

            let subscriber = pos_infra::telemetry::get_subscriber(
                settings.telemetry.service_name.clone(),
                settings.telemetry.log_level.clone(),
                std::io::stdout,
                settings.telemetry.json_logs,
            );
            pos_infra::telemetry::init_subscriber(subscriber);

            let application = Application::build(settings).await?;
            application.run_until_stopped().await?;

            Ok(())
        }
        [_, cmd, flag, username] if cmd == "create-owner" && flag == "--username" => {
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            if line.ends_with("\r\n") {
                line.truncate(line.len() - 2);
            } else if line.ends_with('\n') {
                line.truncate(line.len() - 1);
            }

            let settings = Settings::load()?;
            let pool = xchange::bootstrap::connect(&settings).await?;
            let repo = Arc::new(PgAccountRepository::new(pool));
            let hasher = PasswordHasher::new();
            let service = AccountService::new(repo, hasher);
            let now = std::time::SystemTime::now().into();

            match service.create_owner(username, &line, now).await {
                Ok(owner) => {
                    println!("Owner {} created", owner.username);
                    Ok(())
                }
                Err(e) => {
                    match e {
                        AppError::Domain(sentence) => eprintln!("{sentence}"),
                        other => eprintln!("{other}"),
                    }
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("Invalid command or arguments");
            std::process::exit(1);
        }
    }
}
