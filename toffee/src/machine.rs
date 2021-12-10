use rust_fsm::{StateMachine, StateMachineImpl};
use super::api::{self, message::Content};
use super::message::MessageWriter;

#[derive(Debug)]
pub enum Input {
    Start,
    IncomingMessage(Content),
    RequestInboundTransfer(api::InboundTransferRequest),
    AcceptTransfer,
    RejectTransfer,
    RequestOutboundTransfer(api::OutboundTransferRequest),
    Disconnect,
}

pub enum Output {
    Message(Content),
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

#[derive(Debug)]
pub struct ClientStateMachine {
}

impl StateMachineImpl for ClientStateMachine {
    type Input = Input;
    type State = State;
    type Output = Output;
    const INITIAL_STATE: Self::State = State::Initial;

    fn transition(state: &Self::State, input: &Self::Input) -> Option<Self::State> {
        match (state, input) {
            (State::Initial, Input::Start) => Some(State::Connecting),

            (State::Initial, Input::IncomingMessage(Content::Init(_))) => {
                Some(State::Idle)
            },

            (State::Connecting, Input::IncomingMessage(Content::Init(_))) => {
                Some(State::Idle)
            },

            (State::Idle, Input::IncomingMessage(Content::Disconnect(_))) => {
                Some(State::Disconnected)
            },

            (_, Input::Disconnect) => Some(State::Disconnected),

            // Server transfer negotiation
            (State::Idle, Input::IncomingMessage(Content::InboundTransferRequest(transfer))) => {
                Some(State::InboundTransferOffered(transfer.clone()))
            },

            (State::InboundTransferOffered(transfer), Input::AcceptTransfer) => {
                Some(State::InboundTransferInProgress(transfer.clone()))
            },

            (State::InboundTransferOffered(_), Input::RejectTransfer) => {
                Some(State::Idle)
            },

            // Client transfer negotiation
            (State::Idle, Input::RequestInboundTransfer(transfer)) => {
                Some(State::InboundTransferRequested(transfer.clone()))
            },

            (State::InboundTransferRequested(requestedTransfer), Input::IncomingMessage(Content::AcceptTransfer(acceptedTransfer))) => {
                if requestedTransfer.id == acceptedTransfer.id {
                    Some(State::InboundTransferInProgress(requestedTransfer.clone()))
                } else {
                    Some(state.clone())
                }
            },

            (State::InboundTransferRequested(requestedTransfer), Input::IncomingMessage(Content::RejectTransfer(rejectedTransfer))) => {
                if requestedTransfer.id == rejectedTransfer.id {
                    Some(State::Idle)
                } else {
                    Some(state.clone())
                }
            },

            _ => {
                println!("Machine: invalid input for {:?}: {:?}", state, input);
                None
            },
        }
    }

    fn output(state: &Self::State, input: &Self::Input) -> Option<Self::Output> {
        match (state, input) {
            (State::Initial, Input::IncomingMessage(Content::Init(_))) |
            (State::Initial, Input::Start) => {
                Some(Output::Message(Content::Init(api::Init {
                    version: 1,
                    features: vec![],
                })))
            },

            // Server transfer negotiation
            (State::InboundTransferOffered(transfer), Input::AcceptTransfer) => {
                Some(Output::Message(Content::AcceptTransfer(api::AcceptTransfer { id: transfer.id.clone() })))
            },

            (State::InboundTransferOffered(transfer), Input::RejectTransfer) => {
                Some(Output::Message(Content::RejectTransfer(api::RejectTransfer { id: transfer.id.clone() })))
            },

            // Client transfer negotiation
            (State::Idle, Input::RequestInboundTransfer(transfer)) => {
                Some(Output::Message(Content::InboundTransferRequest(transfer.clone())))
            },

            // ---

            (_, Input::Disconnect) => {
                Some(Output::Message(Content::Disconnect(api::Disconnect { })))
            },
            _ => None
        }
    }
}

pub struct Client<'a> {
    machine: StateMachine<ClientStateMachine>,
    writer: MessageWriter<'a>
}

#[derive(Debug)]
pub enum ClientResult {
    Busy,
    Idle,
    Disconnected,
    InboundTransferOffered(api::InboundTransferRequest),
    OutboundTransferOffered(api::OutboundTransferRequest),
}

pub type Error = Box<dyn std::error::Error>;

impl<'a> Client<'a> {
    pub fn new(writer: MessageWriter<'a>) -> Self {
        Client {
            machine: StateMachine::new(),
            writer,
        }
    }

    pub fn start(&mut self) -> Result<ClientResult, Error> {
        let output = self.machine.consume(&Input::Start);
        self.map_output(output.unwrap())
    }

    fn map_output(&mut self, output: Option<Output>) -> Result<ClientResult, Error> {
        println!("machine now in {:?}", self.machine.state());
        match output {
            Some(Output::Message(msg)) => {
                self.writer.write(msg)?;
            },
            None => {
            },
        }
        match self.machine.state() {
            State::Disconnected => {
                return Ok(ClientResult::Disconnected)
            },
            State::Idle => {
                return Ok(ClientResult::Idle)
            },
            State::InboundTransferOffered(request) => {
                return Ok(ClientResult::InboundTransferOffered(request.clone()))
            },
            State::OutboundTransferOffered(request) => {
                return Ok(ClientResult::OutboundTransferOffered(request.clone()))
            },
            _ => {
                return Ok(ClientResult::Busy)
            }
        }
    }

    pub fn disconnect(&mut self) -> Result<ClientResult, Error> {
        let output = self.machine.consume(&Input::Disconnect).unwrap();
        return self.map_output(output);
    }

    pub fn feed_message(&mut self, msg: Content) -> Result<ClientResult, Error> {
        let output = self.machine.consume(&Input::IncomingMessage(msg))?;
        return self.map_output(output);
    }

    pub fn request_inbound_transfer(&mut self, request: api::InboundTransferRequest) -> Result<ClientResult, Error> {
        let output = self.machine.consume(&Input::RequestInboundTransfer(request))?;
        return self.map_output(output);
    }

    pub fn request_outbound_transfer(&mut self, request: api::OutboundTransferRequest) -> Result<ClientResult, Error> {
        let output = self.machine.consume(&Input::RequestOutboundTransfer(request))?;
        return self.map_output(output);
    }

    pub fn accept_transfer(&mut self) -> Result<ClientResult, Error> {
        let output = self.machine.consume(&Input::AcceptTransfer)?;
        return self.map_output(output);
    }

    pub fn reject_transfer(&mut self) -> Result<ClientResult, Error> {
        let output = self.machine.consume(&Input::RejectTransfer)?;
        return self.map_output(output);
    }
}
