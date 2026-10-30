//! The CLI's side of the control socket: one [`Client`] connection that sends [`Call`]s and
//! waits for each answer, and after `subscribe` reads the event stream.

use std::io::{self, BufReader};

use macaroni_ipc::{Call, Request, Response, Stream};
use serde_json::Value;

/// One connection to the running app.
pub struct Client {
    /// The stream, buffered for reading lines; writes go to the inner stream directly.
    reader: BufReader<Stream>,
    /// The id for the next request. Calls are strictly one at a time, so ids only need to be
    /// unique per connection.
    next_id: u64,
}

impl Client {
    /// Connects to [`macaroni_ipc::endpoint`]. The error says where it looked and suggests the
    /// app isn't running.
    pub fn connect() -> Result<Self, String> {
        let endpoint = macaroni_ipc::endpoint();
        let stream = macaroni_ipc::connect(&endpoint)
            .map_err(|e| format!("can't reach macaroni at {endpoint}: {e} (is it running?)"))?;
        Ok(Self {
            reader: BufReader::new(stream),
            next_id: 1,
        })
    }

    /// Sends one call and blocks for its response. Returns the `result` (`null` if the app sent
    /// none), or the app's `error` message, or a message if the socket fails or closes.
    pub fn call(&mut self, call: Call) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        macaroni_ipc::write_line(&mut self.reader.get_ref(), &Request { id, call })
            .map_err(|e| e.to_string())?;
        // The next line is taken as the answer without checking its id: nothing else is in
        // flight, and events only start after `subscribe`'s own response.
        let response: Response = macaroni_ipc::read_line(&mut self.reader)
            .map_err(|e| e.to_string())?
            .ok_or("macaroni closed the connection")?;
        match response.error {
            Some(error) => Err(error),
            None => Ok(response.result.unwrap_or_default()),
        }
    }

    /// The next streamed event after a `subscribe` call; `None` when the app goes away.
    pub fn next_event(&mut self) -> io::Result<Option<Value>> {
        macaroni_ipc::read_line(&mut self.reader)
    }
}
