use crate::model::{Importer, ImporterConfiguration, ImporterReport, RevisionedImporter};
use actix_web::{body::BoxBody, HttpResponse, ResponseError};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, TransactionTrait,
};
use sea_query::{Alias, Expr, SimpleExpr};
use time::OffsetDateTime;
use trustify_common::model::{Paginated, PaginatedResults, Revisioned};
use trustify_common::{
    db::{Database, DatabaseErrors},
    error::ErrorInformation,
};
use trustify_entity::{importer, importer_report};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("importer '{0}' already exists")]
    AlreadyExists(String),
    #[error("importer '{0}' not found")]
    NotFound(String),
    #[error("mid air collision")]
    MidAirCollision,
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl ResponseError for Error {
    fn error_response(&self) -> HttpResponse<BoxBody> {
        match self {
            Error::AlreadyExists(_) => HttpResponse::Conflict().json(ErrorInformation {
                error: "AlreadyExists".into(),
                message: self.to_string(),
                details: None,
            }),
            Error::NotFound(_) => HttpResponse::NotFound().json(ErrorInformation {
                error: "NotFound".into(),
                message: self.to_string(),
                details: None,
            }),
            Error::MidAirCollision => HttpResponse::PreconditionFailed().json(ErrorInformation {
                error: "MidAirCollision".into(),
                message: self.to_string(),
                details: None,
            }),
            _ => HttpResponse::InternalServerError().json(ErrorInformation {
                error: "Internal".into(),
                message: self.to_string(),
                details: None,
            }),
        }
    }
}

pub struct ImporterService {
    db: Database,
}

impl ImporterService {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn list(&self) -> Result<Vec<Importer>, Error> {
        let result = importer::Entity::find()
            .all(&self.db)
            .await?
            .into_iter()
            .map(Importer::try_from)
            .collect::<Result<_, _>>()?;

        Ok(result)
    }

    pub async fn create(
        &self,
        name: String,
        configuration: ImporterConfiguration,
    ) -> Result<(), Error> {
        let entity = importer::ActiveModel {
            name: Set(name.clone()),
            revision: Set(Uuid::new_v4()),

            state: Set(importer::State::Waiting),
            last_change: Set(OffsetDateTime::now_utc()),

            last_success: Set(None),
            last_run: Set(None),
            last_error: Set(None),

            configuration: Set(serde_json::to_value(configuration)?),
        };

        match entity.insert(&self.db).await {
            Err(err) if err.is_duplicate() => Err(Error::AlreadyExists(name)),
            r => r.map_err(Error::from),
        }?;

        Ok(())
    }

    pub async fn read(&self, name: &str) -> Result<Option<Revisioned<Importer>>, Error> {
        let result = importer::Entity::find_by_id(name).one(&self.db).await?;

        Ok(result
            .map(RevisionedImporter::try_from)
            .transpose()?
            .map(|r| r.0))
    }

    pub async fn update_configuration(
        &self,
        name: &str,
        expected_revision: Option<&str>,
        configuration: ImporterConfiguration,
    ) -> Result<(), Error> {
        self.update(
            &self.db,
            name,
            expected_revision,
            vec![(
                importer::Column::Configuration,
                Expr::value(serde_json::to_value(configuration)?),
            )],
        )
        .await
    }

    /// Update state to indicate the start of an importer run
    pub async fn update_start(
        &self,
        name: &str,
        expected_revision: Option<&str>,
    ) -> Result<(), Error> {
        self.update(
            &self.db,
            name,
            expected_revision,
            vec![
                (
                    importer::Column::LastChange,
                    Expr::value(time::OffsetDateTime::now_utc()),
                ),
                (
                    importer::Column::State,
                    Expr::value(importer::State::Running),
                ),
            ],
        )
        .await
    }

    pub async fn update_finish(
        &self,
        name: &str,
        expected_revision: Option<&str>,
        last_run: OffsetDateTime,
        last_error: Option<String>,
        report: Option<serde_json::Value>,
    ) -> Result<(), Error> {
        let tx = self.db.begin().await?;

        let now = OffsetDateTime::now_utc();
        let successful = last_error.is_none();
        let mut updates = vec![
            (importer::Column::LastError, Expr::value(last_error.clone())),
            (importer::Column::LastRun, Expr::value(last_run)),
            (
                importer::Column::State,
                Expr::value(importer::State::Waiting),
            ),
            (importer::Column::LastChange, Expr::value(now)),
        ];
        if successful {
            updates.push((importer::Column::LastSuccess, Expr::value(now)));
        }

        self.update(&tx, name, expected_revision, updates).await?;

        // add report

        if let Some(report) = report {
            let entity = importer_report::ActiveModel {
                id: Set(Uuid::new_v4()),
                importer: Set(name.to_string()),
                creation: Set(OffsetDateTime::now_utc()),
                error: Set(last_error),
                report: Set(report),
            };
            entity.insert(&tx).await?;
        }

        // commit

        tx.commit().await?;

        Ok(())
    }

    async fn update<C>(
        &self,
        db: &C,
        name: &str,
        expected_revision: Option<&str>,
        updates: Vec<(importer::Column, SimpleExpr)>,
    ) -> Result<(), Error>
    where
        C: ConnectionTrait,
    {
        let mut update = importer::Entity::update_many()
            .col_expr(importer::Column::Revision, Expr::value(Uuid::new_v4()))
            .filter(importer::Column::Name.eq(name));

        for (col, expr) in updates {
            update = update.col_expr(col, expr);
        }

        if let Some(revision) = expected_revision {
            update = update.filter(
                importer::Column::Revision
                    .into_expr()
                    .cast_as(Alias::new("text"))
                    .eq(revision),
            );
        }

        let result = update.exec(db).await?;

        if result.rows_affected == 0 {
            // now we need to figure out if the item wasn't there or if it was modified
            if importer::Entity::find_by_id(name).count(&self.db).await? == 0 {
                Err(Error::NotFound(name.to_string()))
            } else {
                Err(Error::MidAirCollision)
            }
        } else {
            Ok(())
        }
    }

    pub async fn delete(&self, name: &str, expected_revision: Option<&str>) -> Result<bool, Error> {
        let mut delete = importer::Entity::delete_many().filter(importer::Column::Name.eq(name));

        if let Some(revision) = expected_revision {
            delete = delete.filter(
                importer::Column::Revision
                    .into_expr()
                    .cast_as(Alias::new("text"))
                    .eq(revision),
            );
        }

        let result = delete.exec(&self.db).await?;

        Ok(result.rows_affected > 0)
    }

    pub async fn get_reports(
        &self,
        name: &str,
        paginated: Paginated,
    ) -> Result<PaginatedResults<ImporterReport>, Error> {
        let pagination = importer_report::Entity::find()
            .filter(importer_report::Column::Importer.eq(name))
            .order_by_desc(importer_report::Column::Creation)
            .paginate(&self.db, paginated.page_size.get());

        let result = pagination
            .fetch_page(paginated.page)
            .await?
            .into_iter()
            .map(ImporterReport::from)
            .collect();

        Ok(PaginatedResults::new(paginated, result, &pagination).await?)
    }
}
