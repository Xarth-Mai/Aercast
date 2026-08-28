use std::collections::HashMap;

use futures_util::{Stream, StreamExt};

#[derive(Debug, PartialEq)]
pub(super) enum Kind {
    Started,
    Stopped,
    ViewerJoined,
    ViewerLeft,
    Error,
}

pub(super) fn worker(
    connection: zbus::Connection,
    requests: impl Stream<Item = Kind>,
) -> impl Stream<Item = zbus::Result<()>> {
    requests.then(move |kind| {
        let connection = connection.clone();
        async move { send(&connection, kind).await }
    })
}

async fn send(connection: &zbus::Connection, kind: Kind) -> zbus::Result<()> {
    let (summary, body) = match kind {
        Kind::Started => ("Screen sharing started", "Aercast is now sharing."),
        Kind::Stopped => (
            "Screen sharing stopped",
            "The current link is waiting for the next share.",
        ),
        Kind::ViewerJoined => ("Viewer connected", "A Viewer is now watching."),
        Kind::ViewerLeft => (
            "Last Viewer disconnected",
            "No Viewers are currently watching.",
        ),
        Kind::Error => ("Aercast needs attention", "Open Aercast for details."),
    };
    let proxy = zbus::Proxy::new(
        connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await?;
    let _: u32 = proxy
        .call(
            "Notify",
            &(
                "Aercast",
                0_u32,
                "aercast",
                summary,
                body,
                Vec::<&str>::new(),
                HashMap::<&str, zbus::zvariant::Value<'_>>::new(),
                -1_i32,
            ),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use futures_util::TryStreamExt;

    use super::*;

    struct Notifications(Arc<Mutex<Vec<(String, String)>>>);

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Notifications {
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &self,
            app_name: &str,
            replaces_id: u32,
            app_icon: &str,
            summary: &str,
            body: &str,
            actions: Vec<String>,
            hints: HashMap<String, zbus::zvariant::OwnedValue>,
            expire_timeout: i32,
        ) -> u32 {
            assert_eq!((app_name, replaces_id, app_icon), ("Aercast", 0, "aercast"));
            assert!(actions.is_empty() && hints.is_empty());
            assert_eq!(expire_timeout, -1);
            self.0
                .lock()
                .unwrap()
                .push((summary.to_owned(), body.to_owned()));
            1
        }
    }

    #[tokio::test]
    #[ignore = "requires an isolated session bus"]
    async fn sends_the_freedesktop_notification_contract() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let _server = zbus::connection::Builder::session()
            .unwrap()
            .name("org.freedesktop.Notifications")
            .unwrap()
            .serve_at(
                "/org/freedesktop/Notifications",
                Notifications(Arc::clone(&received)),
            )
            .unwrap()
            .build()
            .await
            .unwrap();
        let connection = zbus::Connection::session().await.unwrap();
        let (requests, pending) = iced::futures::channel::mpsc::unbounded();
        requests.unbounded_send(Kind::Started).unwrap();
        requests.unbounded_send(Kind::Stopped).unwrap();
        requests.unbounded_send(Kind::ViewerJoined).unwrap();
        requests.unbounded_send(Kind::ViewerLeft).unwrap();
        drop(requests);
        worker(connection, pending)
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_eq!(
            *received.lock().unwrap(),
            [
                (
                    "Screen sharing started".to_owned(),
                    "Aercast is now sharing.".to_owned(),
                ),
                (
                    "Screen sharing stopped".to_owned(),
                    "The current link is waiting for the next share.".to_owned(),
                ),
                (
                    "Viewer connected".to_owned(),
                    "A Viewer is now watching.".to_owned(),
                ),
                (
                    "Last Viewer disconnected".to_owned(),
                    "No Viewers are currently watching.".to_owned(),
                ),
            ]
        );
    }
}
