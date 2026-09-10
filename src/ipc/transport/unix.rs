use crate::StokerPaths;

pub(super) async fn connect(paths: &StokerPaths) -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(&paths.endpoint).await
}
