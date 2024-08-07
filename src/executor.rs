//! Query executor.

use crate::manager::ResourceManager;
use crate::proto;
use crate::{InfernoError, Result};
use std::rc::Rc;

pub async fn execute_client_req(
    manager: Rc<ResourceManager>,
    req: proto::PipelineReqBody,
    db_name: &str,
    baton: &str,
) -> Result<proto::PipelineRespBody> {
    let mut responses = Vec::new();
    responses
        .try_reserve(req.requests.len())
        .map_err(|_| InfernoError::OutOfMemory)?;
    for req in &req.requests {
        let resp = match req {
            proto::StreamRequest::None => todo!(),
            proto::StreamRequest::Close(_) => exec_close(manager.clone(), db_name, baton).await?,
            proto::StreamRequest::Execute(req) => {
                exec_execute(manager.clone(), &req, db_name, baton).await?
            }
            proto::StreamRequest::Batch(_) => todo!(),
            proto::StreamRequest::Sequence(_) => todo!(),
            proto::StreamRequest::Describe(_) => todo!(),
            proto::StreamRequest::StoreSql(_) => todo!(),
            proto::StreamRequest::CloseSql(_) => todo!(),
            proto::StreamRequest::GetAutocommit(_) => todo!(),
        };
        responses.push(resp);
    }
    return Ok(proto::PipelineRespBody {
        baton: Some(baton.to_string()),
        base_url: None,
        results: responses,
    });
}

async fn exec_close(
    manager: Rc<ResourceManager>,
    db_name: &str,
    baton: &str,
) -> Result<proto::StreamResult> {
    log::trace!("Closing connection: {} (baton = {})", db_name, baton);
    manager.drop_conn(db_name, baton)?;
    Ok(proto::StreamResult::Ok {
        response: proto::StreamResponse::Close(proto::CloseStreamResp {}),
    })
}

async fn exec_execute(
    manager: Rc<ResourceManager>,
    req: &proto::ExecuteStreamReq,
    db_name: &str,
    baton: &str,
) -> Result<proto::StreamResult> {
    log::trace!(
        "Executing SQL statement: {:?} on {} (baton = {}",
        req.stmt,
        db_name,
        baton
    );
    let conn = manager.get_conn(db_name, baton).await?;
    let sql = req.stmt.sql.as_ref().ok_or(InfernoError::InternalError(
        "No SQL statement found".to_string(),
    ))?;
    let rs = conn.query(sql, libsql::params!()).await?;
    let result = make_execute_result(rs).await?;
    Ok(result)
}

async fn make_execute_result(mut rs: libsql::Rows) -> Result<proto::StreamResult> {
    let column_count = rs.column_count();
    let mut cols = Vec::with_capacity(column_count as usize);
    for i in 0..column_count {
        let col = rs
            .column_name(i)
            .ok_or(InfernoError::InternalError(format!(
                "No column name found for column {}",
                i
            )))?;
        let col = proto::Col {
            name: Some(col.to_string()),
            decltype: None, // FIXME
        };
        cols.push(col);
    }
    let mut rows = Vec::new();
    loop {
        match rs.next().await? {
            Some(row) => {
                let mut values = Vec::new();
                for i in 0..column_count {
                    let value = row.get_value(i)?;
                    let value: proto::Value = match value {
                        libsql::Value::Null => proto::Value::Null,
                        libsql::Value::Integer(i) => proto::Value::Integer { value: i },
                        libsql::Value::Real(f) => proto::Value::Float { value: f },
                        libsql::Value::Text(s) => proto::Value::Text { value: s.into() },
                        libsql::Value::Blob(b) => proto::Value::Blob { value: b.into() },
                    };
                    values.push(value);
                }
                rows.push(proto::Row { values });
            }
            None => break,
        }
    }
    let resp = proto::ExecuteStreamResp {
        result: proto::StmtResult {
            cols,
            rows,
            affected_row_count: 0,
            last_insert_rowid: None,
            replication_index: None,
            rows_read: 0,
            rows_written: 0,
            query_duration_ms: 0.0,
        },
    };
    Ok(proto::StreamResult::Ok {
        response: proto::StreamResponse::Execute(resp),
    })
}