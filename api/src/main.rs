use std::io::BufRead;
use std::sync::Arc;

use pos_error::AppError;
use xc_auth::application::account::service::AccountService;
use xc_auth::application::session::SessionService;
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::domain::identity::identity_client::IdentityClient;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_auth::infrastructure::identity::http_identity_client::HttpIdentityClient;
use xchange::bootstrap::Application;
use xchange::config::Settings;
use xchange::presentation::http::client_address::CLIENT_ADDRESS_HEADER;

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

            if let Some(notice) = settings.identity_api_key_notice() {
                tracing::warn!("{notice}");
            }

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
        [_, cmd, name_flag, name, phone_flag, phone]
            if cmd == "create-owner" && name_flag == "--name" && phone_flag == "--phone" =>
        {
            // The password is one line on standard input, never an argument: it stays off the
            // command line and out of the shell history (deploy/README.md, "First owner account").
            let mut password = String::new();
            std::io::stdin().lock().read_line(&mut password)?;
            let password = password.trim_end_matches(['\r', '\n']).to_string();

            let settings = Settings::load()?;
            let identity_settings = settings.identity.clone().ok_or_else(|| {
                anyhow::anyhow!("Identity is not configured (IDENTITY_BASE_URL and its client)")
            })?;
            let identity: Arc<dyn IdentityClient> = Arc::new(HttpIdentityClient::new(
                identity_settings,
                CLIENT_ADDRESS_HEADER,
            ));
            let pool = xchange::bootstrap::connect(&settings).await?;
            let session_service = SessionService::new(pool, Some(identity), None, None);
            let now = chrono::Utc::now();

            match session_service
                .create_owner(name, phone, &password, now)
                .await
            {
                Ok(user_id) => {
                    println!(
                        "Owner {name} created (user {user_id}); sign in on /login with the phone"
                    );
                    Ok(())
                }
                Err(e) => {
                    eprintln!("{e}");
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
