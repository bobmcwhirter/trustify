use crate::progress::init_log_and_progress;
use parking_lot::Mutex;
use sbom_walker::{
    retrieve::RetrievingVisitor,
    source::{DispatchSource, FileSource, HttpOptions, HttpSource},
    validation::ValidationVisitor,
    walker::Walker,
};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::SystemTime;
use time::{Date, Month, UtcOffset};
use trustify_common::{config::Database, db};
use trustify_graph::graph::Graph;
use trustify_module_importer::server::{
    report::{Report, ReportBuilder, ScannerError, SplitScannerError},
    sbom::storage,
};
use url::Url;
use walker_common::{fetcher::Fetcher, progress::Progress, validate::ValidationOptions};

/// Import SBOMs
#[derive(clap::Args, Debug)]
pub struct ImportSbomCommand {
    #[command(flatten)]
    pub database: Database,

    /// GPG key used to sign SBOMs, use the fragment of the URL as fingerprint.
    #[arg(long, env)]
    pub key: Vec<Url>,

    /// Source URL or path
    pub source: String,
}

impl ImportSbomCommand {
    pub async fn run(self) -> anyhow::Result<ExitCode> {
        let progress = init_log_and_progress()?;

        log::info!("Ingesting SBOMs");

        let (report, result) = self.run_once(progress).await.split()?;

        log::info!("Import report: {report:#?}");

        result.map(|()| ExitCode::SUCCESS)
    }

    async fn run_once(self, progress: Progress) -> Result<Report, ScannerError> {
        let report = Arc::new(Mutex::new(ReportBuilder::new()));

        let db = db::Database::with_external_config(&self.database, false).await?;
        let system = Graph::new(db);

        let source: DispatchSource = match Url::parse(&self.source) {
            Ok(url) => {
                let keys = self
                    .key
                    .into_iter()
                    .map(|key| key.into())
                    .collect::<Vec<_>>();
                HttpSource::new(
                    url,
                    Fetcher::new(Default::default()).await?,
                    HttpOptions::new().keys(keys),
                )
                .into()
            }
            Err(_) => FileSource::new(&self.source, None)?.into(),
        };

        // process (called by validator)

        let process = storage::StorageVisitor {
            system,
            report: report.clone(),
        };

        // validate (called by retriever)

        //  because we still have GPG v3 signatures
        let options = ValidationOptions::new().validation_date(SystemTime::from(
            Date::from_calendar_date(2007, Month::January, 1)
                .map_err(|err| ScannerError::Critical(err.into()))?
                .midnight()
                .assume_offset(UtcOffset::UTC),
        ));

        let validation = ValidationVisitor::new(process).with_options(options);

        // retriever (called by filter)

        let visitor = RetrievingVisitor::new(source.clone(), validation);

        // walker

        Walker::new(source)
            .with_progress(progress)
            .walk(visitor)
            .await
            // if the walker fails, we record the outcome as part of the report, but skip any
            // further processing, like storing the marker
            .map_err(|err| ScannerError::Normal {
                err: err.into(),
                report: report.lock().clone().build(),
            })?;

        Ok(match Arc::try_unwrap(report) {
            Ok(report) => report.into_inner(),
            Err(report) => report.lock().clone(),
        }
        .build())
    }
}
