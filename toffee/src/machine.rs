use rust_fsm::{StateMachine, StateMachineImpl};
use super::api::{self, message::Content};
use super::message::MessageWriter;

pub enum Input {
    Start,
    IncomingMessage(Content),
    RequestInboundTransfer(api::InboundTransferRequest),
    RequestOutboundTransfer(api::OutboundTransferRequest),
    Disconnect,
}

pub enum Output {
    Message(Content),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    Initial,
    Connecting,
    Idle,
    Disconnected,
}

#[derive(Debug)]
pub struct ClientMachineMachine {

}

impl StateMachineImpl for ClientMachineMachine {
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
            (State::Idle, Input::RequestInboundTransfer(_)) => Some(State::Idle),
            (State::Idle, Input::IncomingMessage(Content::Disconnect(_))) => {
                Some(State::Disconnected)
            },
            (_, Input::Disconnect) => Some(State::Disconnected),
            _ => None,
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
            (State::Idle, Input::RequestInboundTransfer(request)) =>{
                Some(Output::Message(Content::InboundTransferRequest(request.clone())))
            },
            (_, Input::Disconnect) => {
                Some(Output::Message(Content::Disconnect(api::Disconnect { })))
            },
            _ => None
        }
    }
}

pub struct Client<'a> {
    machine: StateMachine<ClientMachineMachine>,
    writer: MessageWriter<'a>
}

pub enum ClientResult {
    Busy,
    Idle,
    Disconnected,
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
}

// #[test]
// fn circuit_breaker() {
//     let machine: StateMachine<ClientMachineMachine> = StateMachine::new();

//     // Unsuccessful request
//     let machine = Arc::new(Mutex::new(machine));
//     {
//         let mut lock = machine.lock().unwrap();
//         let res = lock.consume(&Input::Unsuccessful).unwrap();
//         assert_eq!(res, Some(ClientMachineOutputSetTimer));
//         assert_eq!(lock.state(), &ClientMachineState::Open);
//     }

//     // Set up a timer
//     let machine_wait = machine.clone();
//     std::thread::spawn(move || {
//         std::thread::sleep(Duration::new(5, 0));
//         let mut lock = machine_wait.lock().unwrap();
//         let res = lock.consume(&ClientMachineInput::TimerTriggered).unwrap();
//         assert_eq!(res, None);
//         assert_eq!(lock.state(), &ClientMachineState::HalfOpen);
//     });

//     // Try to pass a request when the circuit breaker is still open
//     let machine_try = machine.clone();
//     std::thread::spawn(move || {
//         std::thread::sleep(Duration::new(1, 0));
//         let mut lock = machine_try.lock().unwrap();
//         let res = lock.consume(&ClientMachineInput::Successful);
//         assert!(matches!(res, Err(TransitionImpossibleError)));
//         assert_eq!(lock.state(), &ClientMachineState::Open);
//     });

//     // Test if the circit breaker was actually closed
//     std::thread::sleep(Duration::new(7, 0));
//     {
//         let mut lock = machine.lock().unwrap();
//         let res = lock.consume(&ClientMachineInput::Successful).unwrap();
//         assert_eq!(res, None);
//         assert_eq!(lock.state(), &ClientMachineState::Closed);
//     }
// }
