use super::api::{self, message::Content};
use super::message::MessageWriter;

#[derive(Clone, Debug, PartialEq)]
pub struct OpenFile {
    info: api::FileInfo,
}

#[derive(Debug, Clone)]
pub enum Input {
    Start,
    IncomingMessage(Content),
    RequestInboundTransfer(api::InboundTransferRequest),
    AcceptTransfer,
    RejectTransfer,
    RequestOutboundTransfer(api::OutboundTransferRequest),
    OpenFile(api::OpenFile),
    ConfirmFileOpened(api::FileOpened),
    SendChunk(api::Chunk),
    CloseFile,
    CloseTransfer,
    Disconnect,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Initial,
    Connecting,
    Idle,
    InboundTransferOffered(api::InboundTransferRequest),
    InboundTransferRequested(api::InboundTransferRequest),
    InboundTransfer(api::InboundTransferRequest, Option<api::OpenFile>),
    InboundFileTransfer(api::InboundTransferRequest, Option<OpenFile>),
    OutboundTransferOffered(api::OutboundTransferRequest),
    OutboundTransferRequested(api::OutboundTransferRequest),
    OutboundTransferInProgress(api::OutboundTransferRequest, Option<OpenFile>),
    Disconnected,
}

#[derive(Debug)]
pub enum ClientEvent {
    Connected,
    Disconnected,
    InboundTransferOffered(api::InboundTransferRequest),
    OutboundTransferOffered(api::OutboundTransferRequest),
    TransferAccepted(),
    TransferRejected(),
    InboundFileOpening(api::InboundTransferRequest, api::OpenFile),
    FileTransferStarted(OpenFile, api::FileOpened),
    FileClosed(OpenFile),
    TransferClosed,
}

pub struct Client<'a> {
    events: Vec<ClientEvent>,
    state: State,
    writer: MessageWriter<'a>,
}

#[derive(Debug)]
pub struct ClientError {
    pub io_error: Option<std::io::Error>,
    pub state: Option<State>,
    pub input: Option<Input>,
}

impl From<std::io::Error> for ClientError {
    fn from(error: std::io::Error) -> Self {
        ClientError {
            io_error: Some(error),
            state: None,
            input: None,
        }
    }
}

pub type ClientResult = Result<(), ClientError>;

impl<'a> Client<'a> {
    pub fn new(writer: MessageWriter<'a>) -> Self {
        Client {
            events: vec![],
            state: State::Initial,
            writer,
        }
    }

    fn transition(&mut self, state: State) {
        self.state = state;
        println!("machine now in {:?}", self.state);
    }

    fn push_event(&mut self, event: ClientEvent) {
        println!("machine event: {:?}", event);
        self.events.push(event);
    }

    fn consume(&mut self, input: &Input) -> Result<(), ClientError> {
        println!("machine input: {:?}", input);
        match (&self.state, input) {
            (State::Initial, Input::Start) => {
                self.writer.write(Content::Init(api::Init {
                    version: 1,
                    features: vec![],
                }))?;
                self.transition(State::Connecting);
            }

            (State::Initial, Input::IncomingMessage(Content::Init(_))) => {
                self.writer.write(Content::Init(api::Init {
                    version: 1,
                    features: vec![],
                }))?;
                self.push_event(ClientEvent::Connected);
                self.transition(State::Idle);
            }

            (State::Connecting, Input::IncomingMessage(Content::Init(_))) => {
                self.push_event(ClientEvent::Connected);
                self.transition(State::Idle);
            }

            (_, Input::IncomingMessage(Content::Disconnect(_))) => {
                self.push_event(ClientEvent::Disconnected);
                self.transition(State::Disconnected);
            }

            (_, Input::Disconnect) => {
                self.push_event(ClientEvent::Disconnected);
                self.writer.write(Content::Disconnect(api::Disconnect {}))?;
                self.transition(State::Disconnected);
            }

            // Server transfer negotiation
            (State::Idle, Input::IncomingMessage(Content::InboundTransferRequest(transfer))) => {
                self.push_event(ClientEvent::InboundTransferOffered(transfer.clone()));
                self.transition(State::InboundTransferOffered(transfer.clone()));
            }

            (State::InboundTransferOffered(transfer), Input::AcceptTransfer) => {
                self.writer
                    .write(Content::AcceptTransfer(api::AcceptTransfer {}))?;
                let transfer = transfer.clone();
                self.transition(State::InboundTransfer(transfer, None));
            }

            (State::InboundTransferOffered(_), Input::RejectTransfer) => {
                self.writer
                    .write(Content::RejectTransfer(api::RejectTransfer {}))?;
                self.transition(State::Idle);
            }

            (
                State::InboundTransfer(transfer, _),
                Input::IncomingMessage(Content::OpenFile(file)),
            ) => {
                let transfer = transfer.clone();
                self.push_event(ClientEvent::InboundFileOpening(
                    transfer.clone(),
                    file.clone(),
                ));
                self.transition(State::InboundTransfer(transfer, Some(file.clone())));
            }

            (
                State::InboundTransfer(transfer, Some(requested_file)),
                Input::ConfirmFileOpened(file),
            ) => {
                let transfer = transfer.clone();
                let requested_file = requested_file.clone();
                self.writer.write(Content::FileOpened(file.clone()))?;
                self.transition(State::InboundFileTransfer(
                    transfer,
                    Some(OpenFile {
                        info: requested_file.file_info.clone().unwrap(),
                    }),
                ));
            }

            // Client transfer negotiation
            (State::Idle, Input::RequestInboundTransfer(transfer)) => {
                self.transition(State::InboundTransferRequested(transfer.clone()));
                self.writer
                    .write(Content::InboundTransferRequest(transfer.clone()))?;
            }

            (
                State::InboundTransferRequested(requested_transfer),
                Input::IncomingMessage(Content::AcceptTransfer(_)),
            ) => {
                let requested_transfer = requested_transfer.clone();
                self.push_event(ClientEvent::TransferAccepted());
                self.transition(State::InboundTransfer(requested_transfer, None));
            }

            (
                State::InboundTransferRequested(_),
                Input::IncomingMessage(Content::RejectTransfer(_)),
            ) => {
                self.push_event(ClientEvent::TransferRejected());
                self.transition(State::Idle)
            }

            (State::InboundTransfer(transfer, _), Input::OpenFile(file)) => {
                let transfer = transfer.clone();
                let file = file.clone();
                self.writer.write(Content::OpenFile(file.clone()))?;
                self.transition(State::InboundTransfer(transfer.clone(), Some(file)));
            }

            (
                State::InboundTransfer(transfer, Some(requested_file)),
                Input::IncomingMessage(Content::FileOpened(file)),
            ) => {
                let transfer = transfer.clone();
                let requested_file = requested_file.clone();
                let open_file = OpenFile {
                    info: requested_file.file_info.clone().unwrap(),
                };
                self.push_event(ClientEvent::FileTransferStarted(open_file.clone(), file.clone()));
                self.transition(State::InboundFileTransfer(transfer, Some(open_file)));
            }

            // General transfer handling
            (
                State::InboundFileTransfer(transfer, Some(file)),
                Input::CloseFile,
            ) => {
                let transfer = transfer.clone();
                let file = file.clone();
                self.transition(State::InboundFileTransfer(transfer, None));
                self.writer.write(Content::CloseFile(api::CloseFile { }))?;
                self.push_event(ClientEvent::FileClosed(file.clone()));
            }

            (
                State::InboundFileTransfer(transfer, Some(file)),
                Input::IncomingMessage(Content::CloseFile(_)),
            ) => {
                let file = file.clone();
                let transfer = transfer.clone();
                self.transition(State::InboundTransfer(transfer, None));
                self.push_event(ClientEvent::FileClosed(file));
            }

            (
                State::InboundFileTransfer(_, _) |
                State::InboundTransfer(_, _),
                Input::CloseTransfer,
            ) => {
                self.transition(State::Idle);
                self.writer.write(Content::CloseTransfer(api::CloseTransfer { }))?;
                self.push_event(ClientEvent::TransferClosed);
            }

            (
                State::InboundFileTransfer(transfer, _) |
                State::InboundTransfer(transfer, _),
                Input::IncomingMessage(Content::CloseTransfer(_)),
            ) => {
                let transfer = transfer.clone();
                self.transition(State::InboundTransfer(transfer, None));
                self.push_event(ClientEvent::TransferClosed);
            }

            _ => {
                return Err(ClientError {
                    io_error: None,
                    state: Some(self.state.clone()),
                    input: Some(input.clone()),
                })
            }
        }
        Ok(())
    }

    pub fn take_events(&mut self) -> Vec<ClientEvent> {
        let mut events = vec![];
        std::mem::swap(&mut self.events, &mut events);
        return events;
    }

    pub fn start(&mut self) -> ClientResult {
        self.consume(&Input::Start)
    }

    pub fn disconnect(&mut self) -> ClientResult {
        self.consume(&Input::Disconnect)
    }

    pub fn feed_message(&mut self, msg: Content) -> ClientResult {
        self.consume(&Input::IncomingMessage(msg))
    }

    pub fn request_inbound_transfer(
        &mut self,
        request: api::InboundTransferRequest,
    ) -> ClientResult {
        self.consume(&Input::RequestInboundTransfer(request))
    }

    pub fn request_outbound_transfer(
        &mut self,
        request: api::OutboundTransferRequest,
    ) -> ClientResult {
        self.consume(&Input::RequestOutboundTransfer(request))
    }

    pub fn accept_transfer(&mut self) -> ClientResult {
        self.consume(&Input::AcceptTransfer)
    }

    pub fn reject_transfer(&mut self) -> ClientResult {
        self.consume(&Input::RejectTransfer)
    }

    pub fn open_file(&mut self, request: api::OpenFile) -> ClientResult {
        self.consume(&Input::OpenFile(request))
    }

    pub fn confirm_file_opened(&mut self, file: api::FileOpened) -> ClientResult {
        self.consume(&Input::ConfirmFileOpened(file))
    }

    pub fn send_chunk(&mut self, chunk: api::Chunk) -> ClientResult {
        self.consume(&Input::SendChunk(chunk))
    }

    pub fn close_file(&mut self) -> ClientResult {
        self.consume(&Input::CloseFile)
    }

    pub fn close_transfer(&mut self) -> ClientResult {
        self.consume(&Input::CloseTransfer)
    }
}
