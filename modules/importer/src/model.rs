use std::ops::{Deref, DerefMut};
use std::time::Duration;
use time::OffsetDateTime;
use trustify_common::model::Revisioned;
use trustify_entity::importer::Model;
use trustify_entity::{importer, importer_report};
use url::Url;
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct Importer {
    pub name: String,
    #[serde(flatten)]
    pub data: ImporterData,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Waiting,
    Running,
}

impl From<importer::State> for State {
    fn from(value: importer::State) -> Self {
        match value {
            importer::State::Waiting => Self::Waiting,
            importer::State::Running => Self::Running,
        }
    }
}

impl From<State> for importer::State {
    fn from(value: State) -> Self {
        match value {
            State::Waiting => Self::Waiting,
            State::Running => Self::Running,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImporterData {
    pub configuration: ImporterConfiguration,

    /// The current state of the importer
    pub state: State,

    /// The last state change
    #[serde(with = "time::serde::rfc3339")]
    pub last_change: time::OffsetDateTime,

    /// The last successful run
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_success: Option<time::OffsetDateTime>,

    /// The last run (successful or not)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_run: Option<time::OffsetDateTime>,

    /// The error of the last run (empty if successful)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ImporterConfiguration {
    Sbom(SbomImporter),
}

impl Deref for ImporterConfiguration {
    type Target = CommonImporter;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Sbom(importer) => &importer.common,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommonImporter {
    #[serde(default)]
    pub disabled: bool,

    #[serde(with = "humantime_serde")]
    pub period: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SbomImporter {
    #[serde(flatten)]
    pub common: CommonImporter,

    pub source: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<Url>,

    #[serde(default)]
    pub v3_signatures: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only_patterns: Vec<String>,
}

impl Deref for SbomImporter {
    type Target = CommonImporter;

    fn deref(&self) -> &Self::Target {
        &self.common
    }
}

impl DerefMut for SbomImporter {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.common
    }
}

impl TryFrom<Model> for Importer {
    type Error = serde_json::Error;

    fn try_from(
        Model {
            name,
            configuration,
            state,
            last_change,
            last_success,
            last_run,
            last_error,
            revision: _,
        }: Model,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            name,
            data: ImporterData {
                state: state.into(),
                last_change,
                last_success,
                last_run,
                last_error,
                configuration: serde_json::from_value(configuration)?,
            },
        })
    }
}

pub struct RevisionedImporter(pub Revisioned<Importer>);

impl TryFrom<Model> for RevisionedImporter {
    type Error = serde_json::Error;

    fn try_from(
        Model {
            name,
            configuration,
            state,
            last_change,
            last_success,
            last_run,
            last_error,
            revision,
        }: Model,
    ) -> Result<Self, Self::Error> {
        Ok(Self(Revisioned {
            value: Importer {
                name,
                data: ImporterData {
                    state: state.into(),
                    last_change,
                    last_success,
                    last_run,
                    last_error,
                    configuration: serde_json::from_value(configuration)?,
                },
            },
            revision: revision.to_string(),
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImporterReport {
    pub id: String,

    pub importer: String,
    #[serde(with = "time::serde::rfc3339")]
    pub creation: OffsetDateTime,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub report: serde_json::Value,
}

impl From<importer_report::Model> for ImporterReport {
    fn from(value: importer_report::Model) -> Self {
        let importer_report::Model {
            id,
            importer,
            creation,
            error,
            report,
        } = value;
        Self {
            id: id.to_string(),
            importer,
            creation,
            error,
            report,
        }
    }
}
