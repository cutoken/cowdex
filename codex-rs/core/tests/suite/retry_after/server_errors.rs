use super::*;
use pretty_assertions::assert_eq;
use test_case::test_case;

#[derive(Clone, Copy)]
enum Failure {
    Http(u16),
    Stream(&'static str),
    ProcessingError,
}

impl Failure {
    fn response(self) -> ResponseTemplate {
        match self {
            Self::Http(status) => ResponseTemplate::new(status)
                .set_body_json(json!({"error": {"code": "server_error"}})),
            Self::Stream(code) => responses::sse_response(responses::sse_failed(
                "failed",
                code,
                "Temporary server failure",
            )),
            Self::ProcessingError => responses::sse_response(responses::sse(vec![json!({
                "type": "response.failed",
                "response": {
                    "id": "failed", "status": "failed",
                    "error": {
                        "message": "An error occurred while processing your request. You can retry your request, or contact us through our help center at help.openai.com if the error persists. Please include the request ID e50ef8e8-dfd2-4472-bf88-efdbdd17b042 in your message."
                    }
                }
            })])),
        }
    }
}

#[derive(Clone, Copy)]
enum RequestKind {
    Sampling,
    Compaction,
}

#[test_case(Failure::Http(500), RequestKind::Sampling; "http_500_sampling")]
#[test_case(Failure::Http(503), RequestKind::Sampling; "http_503_sampling")]
#[test_case(Failure::Http(502), RequestKind::Compaction; "http_502_compaction")]
#[test_case(Failure::Stream("server_error"), RequestKind::Sampling; "stream_server_error")]
#[test_case(Failure::Stream("internal_server_error"), RequestKind::Sampling; "stream_internal_error")]
#[test_case(Failure::Stream("server_is_overloaded"), RequestKind::Sampling; "stream_capacity")]
#[test_case(Failure::Stream("server_error"), RequestKind::Compaction; "stream_compaction")]
#[test_case(Failure::Stream("server_is_overloaded"), RequestKind::Compaction; "compaction_capacity")]
#[test_case(Failure::ProcessingError, RequestKind::Sampling; "processing_error_without_code")]
#[test_case(Failure::ProcessingError, RequestKind::Compaction; "compaction_processing_error_without_code")]
#[tokio::test(flavor = "current_thread")]
async fn server_errors_recover_beyond_old_limits_at_fixed_sixty_seconds(
    failure: Failure,
    request: RequestKind,
) -> Result<()> {
    let mut telemetry = RetryTelemetryCapture::install();
    let server = responses::start_mock_server().await;
    let recovered = responses::sse_response(match request {
        RequestKind::Sampling => responses::sse_completed("recovered"),
        RequestKind::Compaction => responses::sse(vec![
            json!({
                "type": "response.output_item.done",
                "item": {"type": "compaction", "encrypted_content": "COMPACTED"}
            }),
            responses::ev_completed("recovered"),
        ]),
    });
    let mock = responses::mount_response_sequence(
        &server,
        std::iter::once(responses::sse_response(responses::sse_completed("seed")))
            .chain((0..25).map(|_| failure.response()))
            .chain([recovered])
            .collect(),
    )
    .await;
    let test = test_codex()
        .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
        .with_config(|config| {
            config.model_provider.request_max_retries = Some(0);
            config.model_provider.stream_max_retries = Some(0);
        })
        .build_with_auto_env(&server)
        .await?;
    test.submit_text_turn("seed history").await?;
    match request {
        RequestKind::Sampling => submit_user_input(&test, "continue through the outage").await?,
        RequestKind::Compaction => {
            test.codex.submit(Op::Compact).await?;
        }
    }
    for attempt in 1..=25 {
        let retry = telemetry.next_retry().await;
        if attempt == 1 && matches!(failure, Failure::Stream(_) | Failure::ProcessingError) {
            let EventMsg::StreamError(error) = wait_for_event(&test.codex, |event| {
                matches!(event, EventMsg::StreamError(_))
            })
            .await
            else {
                unreachable!()
            };
            insta::assert_snapshot!("unlimited_server_retry_message", error.message);
        }
        let (layer, operation) = match (failure, request) {
            (Failure::Http(_), _) => ("http", "request"),
            (Failure::Stream(_) | Failure::ProcessingError, RequestKind::Sampling) => {
                ("stream", "sampling")
            }
            (Failure::Stream(_) | Failure::ProcessingError, RequestKind::Compaction) => {
                ("stream", "remote_compaction_v2")
            }
        };
        assert_eq!(
            retry,
            RetryTelemetryEvent {
                attempt,
                delay: Duration::from_secs(60),
                layer: layer.into(),
                operation: operation.into(),
            }
        );
        tokio::time::pause();
        tokio::time::advance(retry.delay).await;
        tokio::time::resume();
    }
    wait_for_turn_completion(&test).await;
    assert_eq!(mock.requests().len(), 27);
    Ok(())
}

#[test_case(Failure::Http(500); "http")]
#[test_case(Failure::Stream("server_is_overloaded"); "stream")]
#[tokio::test(flavor = "current_thread")]
async fn server_retry_sleep_can_be_interrupted(failure: Failure) -> Result<()> {
    let mut telemetry = RetryTelemetryCapture::install();
    let server = responses::start_mock_server().await;
    let mock = responses::mount_response_once(&server, failure.response()).await;
    let test = test_codex().build_with_auto_env(&server).await?;
    submit_user_input(&test, "wait through the outage").await?;
    assert_eq!(telemetry.next_retry().await.delay, Duration::from_secs(60));
    test.codex.submit(Op::Interrupt).await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnAborted(_))
    })
    .await;
    assert_eq!(mock.requests().len(), 1);
    Ok(())
}
