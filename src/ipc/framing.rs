//! Length-delimited JSON framing for the versioned IPC protocol.

use futures_util::SinkExt;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use super::{IPC_VERSION, IpcRequest, IpcResponse, ProtocolVersionMismatch};

#[derive(Debug, Serialize, Deserialize)]
struct VersionedRequest {
    version: u16,
    request: IpcRequest,
}

#[derive(Debug, Serialize, Deserialize)]
struct VersionedResponse {
    version: u16,
    response: IpcResponse,
}

pub(crate) fn encode_request(request: &IpcRequest) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&VersionedRequest {
        version: IPC_VERSION,
        request: request.clone(),
    })?)
}

pub(crate) fn decode_request(frame: &[u8]) -> anyhow::Result<IpcRequest> {
    let message: VersionedRequest = serde_json::from_slice(frame)?;
    ensure_version(message.version)?;
    Ok(message.request)
}

pub(crate) fn encode_response(response: &IpcResponse) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&VersionedResponse {
        version: IPC_VERSION,
        response: response.clone(),
    })?)
}

pub(crate) fn decode_response(frame: &[u8]) -> anyhow::Result<IpcResponse> {
    let message: VersionedResponse = serde_json::from_slice(frame)?;
    ensure_version(message.version)?;
    Ok(message.response)
}

fn ensure_version(actual: u16) -> anyhow::Result<()> {
    if actual == IPC_VERSION {
        return Ok(());
    }
    Err(ProtocolVersionMismatch {
        expected: IPC_VERSION,
        actual,
    }
    .into())
}

pub(crate) async fn send_response<S>(
    framed: &mut Framed<S, LengthDelimitedCodec>,
    response: &IpcResponse,
) -> anyhow::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    framed.send(encode_response(response)?.into()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_mismatch_is_a_typed_failure() {
        let frame = serde_json::json!({ "version": IPC_VERSION - 1, "request": "Status" });
        let error = decode_request(&serde_json::to_vec(&frame).unwrap()).unwrap_err();
        let mismatch = error.downcast_ref::<ProtocolVersionMismatch>().unwrap();
        assert_eq!(mismatch.expected, IPC_VERSION);
        assert_eq!(mismatch.actual, IPC_VERSION - 1);
    }
}
