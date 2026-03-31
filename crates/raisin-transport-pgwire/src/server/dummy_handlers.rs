// SPDX-License-Identifier: BSL-1.1

//! Dummy handler implementations for static assertions and testing.

use pgwire::api::PgWireHandlerFactory;
use std::sync::Arc;

// Ensure server can be safely sent between threads
static_assertions::assert_impl_all!(super::PgWireServer<DummyHandler>: Send);

pub(super) struct DummyHandler;
pub(super) struct DummyStartup;
pub(super) struct DummySimpleQuery;
pub(super) struct DummyExtendedQuery;
pub(super) struct DummyCopy;

#[async_trait::async_trait]
impl pgwire::api::auth::StartupHandler for DummyStartup {
    async fn on_startup<C>(
        &self,
        _client: &mut C,
        _message: pgwire::messages::PgWireFrontendMessage,
    ) -> std::result::Result<(), pgwire::error::PgWireError>
    where
        C: pgwire::api::ClientInfo
            + futures::sink::Sink<pgwire::messages::PgWireBackendMessage>
            + Unpin
            + Send,
        C::Error: std::fmt::Debug,
        pgwire::error::PgWireError:
            From<<C as futures::sink::Sink<pgwire::messages::PgWireBackendMessage>>::Error>,
    {
        Ok(())
    }
}

#[async_trait::async_trait]
impl pgwire::api::query::SimpleQueryHandler for DummySimpleQuery {
    async fn do_query<'a, 'b: 'a, C>(
        &'b self,
        _client: &mut C,
        _query: &'a str,
    ) -> std::result::Result<Vec<pgwire::api::results::Response<'a>>, pgwire::error::PgWireError>
    where
        C: pgwire::api::ClientInfo
            + futures::sink::Sink<pgwire::messages::PgWireBackendMessage>
            + Unpin
            + Send
            + Sync,
        C::Error: std::fmt::Debug,
        pgwire::error::PgWireError:
            From<<C as futures::sink::Sink<pgwire::messages::PgWireBackendMessage>>::Error>,
    {
        Ok(vec![])
    }
}

#[async_trait::async_trait]
impl pgwire::api::query::ExtendedQueryHandler for DummyExtendedQuery {
    type Statement = String;
    type QueryParser = pgwire::api::stmt::NoopQueryParser;

    fn query_parser(&self) -> Arc<Self::QueryParser> {
        Arc::new(pgwire::api::stmt::NoopQueryParser)
    }

    async fn do_query<'a, 'b: 'a, C>(
        &'b self,
        _client: &mut C,
        _portal: &'a pgwire::api::portal::Portal<Self::Statement>,
        _max_rows: usize,
    ) -> std::result::Result<pgwire::api::results::Response<'a>, pgwire::error::PgWireError>
    where
        C: pgwire::api::ClientInfo
            + pgwire::api::ClientPortalStore
            + futures::sink::Sink<pgwire::messages::PgWireBackendMessage>
            + Unpin
            + Send
            + Sync,
        C::PortalStore: pgwire::api::store::PortalStore<Statement = Self::Statement>,
        C::Error: std::fmt::Debug,
        pgwire::error::PgWireError:
            From<<C as futures::sink::Sink<pgwire::messages::PgWireBackendMessage>>::Error>,
    {
        Ok(pgwire::api::results::Response::EmptyQuery)
    }

    async fn do_describe_statement<C>(
        &self,
        _client: &mut C,
        _statement: &pgwire::api::stmt::StoredStatement<Self::Statement>,
    ) -> std::result::Result<
        pgwire::api::results::DescribeStatementResponse,
        pgwire::error::PgWireError,
    >
    where
        C: pgwire::api::ClientInfo
            + pgwire::api::ClientPortalStore
            + futures::sink::Sink<pgwire::messages::PgWireBackendMessage>
            + Unpin
            + Send
            + Sync,
        C::PortalStore: pgwire::api::store::PortalStore<Statement = Self::Statement>,
        C::Error: std::fmt::Debug,
        pgwire::error::PgWireError:
            From<<C as futures::sink::Sink<pgwire::messages::PgWireBackendMessage>>::Error>,
    {
        Ok(pgwire::api::results::DescribeStatementResponse::new(
            vec![],
            vec![],
        ))
    }

    async fn do_describe_portal<C>(
        &self,
        _client: &mut C,
        _portal: &pgwire::api::portal::Portal<Self::Statement>,
    ) -> std::result::Result<pgwire::api::results::DescribePortalResponse, pgwire::error::PgWireError>
    where
        C: pgwire::api::ClientInfo
            + pgwire::api::ClientPortalStore
            + futures::sink::Sink<pgwire::messages::PgWireBackendMessage>
            + Unpin
            + Send
            + Sync,
        C::PortalStore: pgwire::api::store::PortalStore<Statement = Self::Statement>,
        C::Error: std::fmt::Debug,
        pgwire::error::PgWireError:
            From<<C as futures::sink::Sink<pgwire::messages::PgWireBackendMessage>>::Error>,
    {
        Ok(pgwire::api::results::DescribePortalResponse::new(vec![]))
    }
}

#[async_trait::async_trait]
impl pgwire::api::copy::CopyHandler for DummyCopy {
    async fn on_copy_data<C>(
        &self,
        _client: &mut C,
        _copy_data: pgwire::messages::copy::CopyData,
    ) -> std::result::Result<(), pgwire::error::PgWireError>
    where
        C: pgwire::api::ClientInfo
            + futures::sink::Sink<pgwire::messages::PgWireBackendMessage>
            + Unpin
            + Send
            + Sync,
        C::Error: std::fmt::Debug,
        pgwire::error::PgWireError:
            From<<C as futures::sink::Sink<pgwire::messages::PgWireBackendMessage>>::Error>,
    {
        Ok(())
    }

    async fn on_copy_done<C>(
        &self,
        _client: &mut C,
        _done: pgwire::messages::copy::CopyDone,
    ) -> std::result::Result<(), pgwire::error::PgWireError>
    where
        C: pgwire::api::ClientInfo
            + futures::sink::Sink<pgwire::messages::PgWireBackendMessage>
            + Unpin
            + Send
            + Sync,
        C::Error: std::fmt::Debug,
        pgwire::error::PgWireError:
            From<<C as futures::sink::Sink<pgwire::messages::PgWireBackendMessage>>::Error>,
    {
        Ok(())
    }

    async fn on_copy_fail<C>(
        &self,
        _client: &mut C,
        fail: pgwire::messages::copy::CopyFail,
    ) -> pgwire::error::PgWireError
    where
        C: pgwire::api::ClientInfo
            + futures::sink::Sink<pgwire::messages::PgWireBackendMessage>
            + Unpin
            + Send
            + Sync,
        C::Error: std::fmt::Debug,
        pgwire::error::PgWireError:
            From<<C as futures::sink::Sink<pgwire::messages::PgWireBackendMessage>>::Error>,
    {
        use pgwire::error::ErrorInfo;
        pgwire::error::PgWireError::UserError(Box::new(ErrorInfo::new(
            "ERROR".to_owned(),
            "XX000".to_owned(),
            format!("COPY mode terminated: {}", fail.message),
        )))
    }
}

impl PgWireHandlerFactory for DummyHandler {
    type StartupHandler = DummyStartup;
    type SimpleQueryHandler = DummySimpleQuery;
    type ExtendedQueryHandler = DummyExtendedQuery;
    type CopyHandler = DummyCopy;

    fn simple_query_handler(&self) -> Arc<Self::SimpleQueryHandler> {
        Arc::new(DummySimpleQuery)
    }

    fn extended_query_handler(&self) -> Arc<Self::ExtendedQueryHandler> {
        Arc::new(DummyExtendedQuery)
    }

    fn startup_handler(&self) -> Arc<Self::StartupHandler> {
        Arc::new(DummyStartup)
    }

    fn copy_handler(&self) -> Arc<Self::CopyHandler> {
        Arc::new(DummyCopy)
    }
}
