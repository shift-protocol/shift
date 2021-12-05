use rust_fsm::{StateMachine, StateMachineImpl};
use super::api::{self, message::Content};
use super::message::{MessageReader, MessageWriter};

pub enum Input {
    Start,
    Message(Content),
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
    const INITIAL_STATE: Self::State = State::Connecting;

    fn transition(state: &Self::State, input: &Self::Input) -> Option<Self::State> {
        match (state, input) {
            (State::Initial, Input::Start) => Some(State::Connecting),
            (State::Connecting, Input::Message(Content::ServerInit(msg))) => {
                Some(State::Idle)
            },
            (_, Input::Disconnect) => {
                Some(State::Disconnected)
            },
            _ => None,
        }
    }

    fn output(state: &Self::State, input: &Self::Input) -> Option<Self::Output> {
        match (state, input) {
            (State::Initial, Input::Start) => {
                Some(Output::Message(Content::ClientInit(api::ClientInit {
                    version: 1,
                    features: vec![],
                })))
            },
            _ => None
        }
    }
}

pub struct Client {
    machine: StateMachine<ClientMachineMachine>,
}

pub enum ClientResult {
    Message(Content),
    Busy,
    Idle,
    Disconnected,
}

impl Client {
    pub fn new() -> Self {
        Client {
            machine: StateMachine::new(),
        }
    }

    pub fn feed_message(&mut self, msg: Content) -> Result<ClientResult, Box<dyn std::error::Error>> {
        let output = self.machine.consume(&Input::Message(msg))?;
        match output {
            Some(Output::Message(msg)) => {
                return Ok(ClientResult::Message(msg));
            },
            None => {
                match self.machine.state() {
                    State::Disconnected => {
                        return Ok(ClientResult::Disconnected),
                    },
                    State::Idle => {
                        return Ok(ClientResult::Idle),
                    },
                    _ => {
                        return Ok(ClientResult::Busy);
                    }
                }
            },
        }
        Ok(ClientResult::Busy)
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
