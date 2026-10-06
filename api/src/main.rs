use xchange::bootstrap::Application;
use xchange::config::Settings;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
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
