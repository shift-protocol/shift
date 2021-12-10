use super::api::{self, message::Content};
use super::message::MessageWriter;

#[derive(Debug, Clone)]
pub enum Input {
    Start,
    IncomingMessage(Content),
    RequestInboundTransfer(api::InboundTransferRequest),
    AcceptTransfer,
    RejectTransfer,
    RequestOutboundTransfer(api::OutboundTransferRequest),
    Disconnect,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Initial,
    Connecting,
    Idle,
    InboundTransferOffered(api::InboundTransferRequest),
    InboundTransferRequested(api::InboundTransferRequest),
    InboundTransferInProgress(api::InboundTransferRequest),
    OutboundTransferOffered(api::OutboundTransferRequest),
    OutboundTransferRequested(api::OutboundTransferRequest),
    OutboundTransferInProgress(api::OutboundTransferRequest),
    Disconnected,
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
            },

            (State::Initial, Input::IncomingMessage(Content::Init(_))) => {
                self.writer.write(Content::Init(api::Init {
                    version: 1,
                    features: vec![],
                }))?;
                self.push_event(ClientEvent::Connected);
                self.transition(State::Idle);
            },

            (State::Connecting, Input::IncomingMessage(Content::Init(_))) => {
                self.push_event(ClientEvent::Connected);
                self.transition(State::Idle);
            },

            (State::Idle, Input::IncomingMessage(Content::Disconnect(_))) => {
                self.transition(State::Disconnected);
            },

            (_, Input::Disconnect) => {
                self.push_event(ClientEvent::Disconnected);
                self.writer.write(Content::Disconnect(api::Disconnect { }))?;
                self.transition(State::Disconnected);
            },

            // Server transfer negotiation
            (State::Idle, Input::IncomingMessage(Content::InboundTransferRequest(transfer))) => {
                self.push_event(ClientEvent::InboundTransferOffered(transfer.clone()));
                self.transition(State::InboundTransferOffered(transfer.clone()));
            },

            (State::InboundTransferOffered(transfer), Input::AcceptTransfer) => {
                self.writer.write(Content::AcceptTransfer(api::AcceptTransfer { id: transfer.id.clone() }))?;
                let transfer = transfer.clone();
                self.transition(State::InboundTransferInProgress(transfer));
            },

            (State::InboundTransferOffered(transfer), Input::RejectTransfer) => {
                self.writer.write(Content::RejectTransfer(api::RejectTransfer { id: transfer.id.clone() }))?;
                self.transition(State::Idle);
            },

            // Client transfer negotiation
            (State::Idle, Input::RequestInboundTransfer(transfer)) => {
                self.transition(State::InboundTransferRequested(transfer.clone()));
                self.writer.write(Content::InboundTransferRequest(transfer.clone()))?;
            },

            (State::InboundTransferRequested(requested_transfer), Input::IncomingMessage(Content::AcceptTransfer(accepted_transfer))) => {
                if requested_transfer.id == accepted_transfer.id {
                    let id = accepted_transfer.id.clone();
                    let requested_transfer = requested_transfer.clone();
                    self.push_event(ClientEvent::TransferAccepted(id));
                    self.transition(State::InboundTransferInProgress(requested_transfer));
                } else {
                    println!("transfer id mismatch");
                }
            },

            (State::InboundTransferRequested(requested_transfer), Input::IncomingMessage(Content::RejectTransfer(rejected_transfer))) => {
                if requested_transfer.id == rejected_transfer.id {
                    self.push_event(ClientEvent::TransferRejected(rejected_transfer.id.clone()));
                    self.transition(State::Idle)
                } else {
                    println!("transfer id mismatch");
                }
            },

            _ => {
                return Err(ClientError { io_error: None, state: Some(self.state.clone()), input: Some(input.clone()) })
            },
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

    pub fn request_inbound_transfer(&mut self, request: api::InboundTransferRequest) -> ClientResult {
        self.consume(&Input::RequestInboundTransfer(request))
    }

    pub fn request_outbound_transfer(&mut self, request: api::OutboundTransferRequest) -> ClientResult {
        self.consume(&Input::RequestOutboundTransfer(request))
    }

    pub fn accept_transfer(&mut self) -> ClientResult {
        self.consume(&Input::AcceptTransfer)
    }

    pub fn reject_transfer(&mut self) -> ClientResult {
        self.consume(&Input::RejectTransfer)
    }
}

#[derive(Debug)]
pub enum ClientEvent {
    Connected,
    Disconnected,
    InboundTransferOffered(api::InboundTransferRequest),
    OutboundTransferOffered(api::OutboundTransferRequest),
    TransferAccepted(String),
    TransferRejected(String),
}
