use super::api::{self, message::Content};
use super::message::MessageWriter;
use anyhow::{bail, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct OpenFile {
    pub info: api::FileInfo,
}

#[derive(Debug, Clone)]
pub enum Input {
    Start,
    IncomingMessage(Content),
    RequestInboundTransfer(api::ReceiveRequest),
    AcceptTransfer,
    RejectTransfer,
    RequestOutboundTransfer(api::SendRequest),
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
    InboundTransferRequested(api::ReceiveRequest),
    InboundTransferOffered(api::SendRequest),
    InboundTransfer(api::SendRequest, Option<api::OpenFile>),
    InboundFileTransfer(api::SendRequest, Option<OpenFile>),
    OutboundTransferRequested(api::SendRequest),
    OutboundTransfer(api::SendRequest, Option<api::OpenFile>),
    OutboundFileTransfer(api::SendRequest, Option<OpenFile>),
    Disconnected,
}

#[derive(Debug)]
pub enum ShiftClientEvent {
    Connected,
    Disconnected,
    InboundTransferOffered(api::SendRequest),
    OutboundTransferOffered(api::ReceiveRequest),
    TransferAccepted(),
    TransferRejected(),
    InboundFileOpening(api::SendRequest, api::OpenFile),
    FileTransferStarted(OpenFile, api::FileOpened),
    Chunk(api::Chunk),
    FileClosed(OpenFile),
    TransferClosed,
}

pub struct ShiftClient<'a> {
    events: Vec<ShiftClientEvent>,
    state: State,
    writer: MessageWriter<'a>,
}

#[derive(thiserror::Error, Debug)]
#[error("Invalid transition")]
pub enum ClientError {
    InvalidTransitionError { state: State, input: Input },
    InvalidStateError(&'static str),
}

impl<'a> ShiftClient<'a> {
    pub fn new(writer: MessageWriter<'a>) -> Self {
        ShiftClient {
            events: vec![],
            state: State::Initial,
            writer,
        }
    }

    fn transition(&mut self, state: State) {
        self.state = state;
        // println!("machine now in {:?}", self.state);
    }

    fn push_event(&mut self, event: ShiftClientEvent) {
        // println!("machine event: {:?}", event);
        self.events.push(event);
    }

    fn consume(&mut self, input: Input) -> Result<()> {
        // println!("machine input: {:?}", input);
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
                self.push_event(ShiftClientEvent::Connected);
                self.transition(State::Idle);
            }

            (State::Connecting, Input::IncomingMessage(Content::Init(_))) => {
                self.push_event(ShiftClientEvent::Connected);
                self.transition(State::Idle);
            }

            (_, Input::IncomingMessage(Content::Disconnect(_))) => {
                self.push_event(ShiftClientEvent::Disconnected);
                self.transition(State::Disconnected);
            }

            (_, Input::Disconnect) => {
                self.push_event(ShiftClientEvent::Disconnected);
                self.writer.write(Content::Disconnect(api::Disconnect {}))?;
                self.transition(State::Disconnected);
            }

            // Inbound transfer handling
            (State::Idle, Input::RequestInboundTransfer(transfer)) => {
                self.transition(State::InboundTransferRequested(transfer.clone()));
                self.writer.write(Content::ReceiveRequest(transfer))?;
            }

            (
                State::Idle | State::InboundTransferRequested(_),
                Input::IncomingMessage(Content::SendRequest(transfer)),
            ) => {
                self.push_event(ShiftClientEvent::InboundTransferOffered(transfer.clone()));
                self.transition(State::InboundTransferOffered(transfer));
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
                self.push_event(ShiftClientEvent::InboundFileOpening(
                    transfer.clone(),
                    file.clone(),
                ));
                self.transition(State::InboundTransfer(transfer, Some(file)));
            }

            (
                State::InboundTransfer(transfer, Some(requested_file)),
                Input::ConfirmFileOpened(file),
            ) => {
                let transfer = transfer.clone();
                let requested_file = requested_file.clone();
                self.writer.write(Content::FileOpened(file))?;
                self.transition(State::InboundFileTransfer(
                    transfer,
                    Some(OpenFile {
                        info: requested_file
                            .file_info
                            .ok_or(ClientError::InvalidStateError(
                                "Missing open file information",
                            ))?,
                    }),
                ));
            }

            // Outbound transfer handling
            (State::Idle, Input::IncomingMessage(Content::ReceiveRequest(request))) => {
                self.push_event(ShiftClientEvent::OutboundTransferOffered(request));
            }

            (State::Idle, Input::RequestOutboundTransfer(transfer)) => {
                self.transition(State::OutboundTransferRequested(transfer.clone()));
                self.writer.write(Content::SendRequest(transfer))?;
            }

            (
                State::OutboundTransferRequested(requested_transfer),
                Input::IncomingMessage(Content::AcceptTransfer(_)),
            ) => {
                let requested_transfer = requested_transfer.clone();
                self.push_event(ShiftClientEvent::TransferAccepted());
                self.transition(State::OutboundTransfer(requested_transfer, None));
            }

            (
                State::OutboundTransferRequested(_),
                Input::IncomingMessage(Content::RejectTransfer(_)),
            ) => {
                self.push_event(ShiftClientEvent::TransferRejected());
                self.transition(State::Idle)
            }

            (State::OutboundTransfer(transfer, _), Input::OpenFile(file)) => {
                let transfer = transfer.clone();
                self.writer.write(Content::OpenFile(file.clone()))?;
                self.transition(State::OutboundTransfer(transfer, Some(file)));
            }

            (
                State::OutboundTransfer(transfer, Some(requested_file)),
                Input::IncomingMessage(Content::FileOpened(file)),
            ) => {
                let transfer = transfer.clone();
                let requested_file = requested_file.clone();
                let open_file = OpenFile {
                    info: requested_file
                        .file_info
                        .ok_or(ClientError::InvalidStateError(
                            "Missing file info in request",
                        ))?,
                };
                self.push_event(ShiftClientEvent::FileTransferStarted(
                    open_file.clone(),
                    file,
                ));
                self.transition(State::OutboundFileTransfer(transfer, Some(open_file)));
            }

            // General transfer handling
            (State::OutboundFileTransfer(_, _), Input::SendChunk(chunk)) => {
                self.writer.write(Content::Chunk(chunk))?;
            }

            (State::InboundFileTransfer(_, _), Input::IncomingMessage(Content::Chunk(chunk))) => {
                self.push_event(ShiftClientEvent::Chunk(chunk));
            }

            (State::OutboundFileTransfer(transfer, Some(file)), Input::CloseFile) => {
                let transfer = transfer.clone();
                let file = file.clone();
                self.transition(State::OutboundTransfer(transfer, None));
                self.writer.write(Content::CloseFile(api::CloseFile {}))?;
                self.push_event(ShiftClientEvent::FileClosed(file));
            }

            (
                State::InboundFileTransfer(transfer, Some(file)),
                Input::IncomingMessage(Content::CloseFile(_)),
            ) => {
                let file = file.clone();
                let transfer = transfer.clone();
                self.transition(State::InboundTransfer(transfer, None));
                self.push_event(ShiftClientEvent::FileClosed(file));
            }

            (
                State::InboundFileTransfer(_, _)
                | State::InboundTransfer(_, _)
                | State::OutboundFileTransfer(_, _)
                | State::OutboundTransfer(_, _),
                Input::CloseTransfer,
            ) => {
                self.transition(State::Idle);
                self.writer
                    .write(Content::CloseTransfer(api::CloseTransfer {}))?;
                self.push_event(ShiftClientEvent::TransferClosed);
            }

            (
                State::InboundFileTransfer(_, _)
                | State::InboundTransfer(_, _)
                | State::OutboundFileTransfer(_, _)
                | State::OutboundTransfer(_, _),
                Input::IncomingMessage(Content::CloseTransfer(_)),
            ) => {
                self.transition(State::Idle);
                self.push_event(ShiftClientEvent::TransferClosed);
            }

            (
                _,
                Input::CloseTransfer
                | Input::CloseFile
                | Input::IncomingMessage(Content::CloseTransfer(_))
                | Input::IncomingMessage(Content::CloseFile(_)),
            ) => {}

            (_, input) => {
                bail!(ClientError::InvalidTransitionError {
                    state: self.state.clone(),
                    input,
                });
            }
        }
        Ok(())
    }

    pub fn take_events(&mut self) -> Vec<ShiftClientEvent> {
        let mut events = vec![];
        std::mem::swap(&mut self.events, &mut events);
        events
    }

    pub fn start(&mut self) -> Result<()> {
        self.consume(Input::Start)
    }

    pub fn disconnect(&mut self) -> Result<()> {
        self.consume(Input::Disconnect)
    }

    pub fn feed_message(&mut self, msg: Content) -> Result<()> {
        self.consume(Input::IncomingMessage(msg))
    }

    pub fn request_inbound_transfer(&mut self, request: api::ReceiveRequest) -> Result<()> {
        self.consume(Input::RequestInboundTransfer(request))
    }

    pub fn request_outbound_transfer(&mut self, request: api::SendRequest) -> Result<()> {
        self.consume(Input::RequestOutboundTransfer(request))
    }

    pub fn accept_transfer(&mut self) -> Result<()> {
        self.consume(Input::AcceptTransfer)
    }

    pub fn reject_transfer(&mut self) -> Result<()> {
        self.consume(Input::RejectTransfer)
    }

    pub fn open_file(&mut self, request: api::OpenFile) -> Result<()> {
        self.consume(Input::OpenFile(request))
    }

    pub fn confirm_file_opened(&mut self, file: api::FileOpened) -> Result<()> {
        self.consume(Input::ConfirmFileOpened(file))
    }

    pub fn send_chunk(&mut self, chunk: api::Chunk) -> Result<()> {
        self.consume(Input::SendChunk(chunk))
    }

    pub fn close_file(&mut self) -> Result<()> {
        self.consume(Input::CloseFile)
    }

    pub fn close_transfer(&mut self) -> Result<()> {
        self.consume(Input::CloseTransfer)
    }
}
