use rust_fsm::StateMachine;
use super::api::message::Content;
use super::

pub enum ClientMachineInput {
    Message(Content),
    Disconnect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClientMachineState {
    Connecting,
    Idle,
    Disconnected,
}

#[derive(Debug)]
pub struct ClientMachineMachine {

}

impl StateMachineImpl for ClientMachineMachine {
    type Input = ClientMachineInput;
    type State = ClientMachineState;
    type Output = ();
    const INITIAL_STATE: Self::State = ClientMachineState::Connecting;

    fn transition(state: &Self::State, input: &Self::Input) -> Option<Self::State> {
        match (state, input) {
            (ClientMachineState::Closed, ClientMachineInput::Unsuccessful) => {
                Some(ClientMachineState::Open)
            }
            (ClientMachineState::Open, ClientMachineInput::TimerTriggered) => {
                Some(ClientMachineState::HalfOpen)
            }
            (ClientMachineState::HalfOpen, ClientMachineInput::Successful) => {
                Some(ClientMachineState::Closed)
            }
            (ClientMachineState::HalfOpen, ClientMachineInput::Unsuccessful) => {
                Some(ClientMachineState::Open)
            }
            _ => None,
        }
    }

    fn output(state: &Self::State, input: &Self::Input) -> Option<Self::Output> {
        match (state, input) {
            (ClientMachineState::Closed, ClientMachineInput::Unsuccessful) => {
                Some(ClientMachineOutputSetTimer)
            }
            (ClientMachineState::HalfOpen, ClientMachineInput::Unsuccessful) => {
                Some(ClientMachineOutputSetTimer)
            }
            _ => None,
        }
    }
}

#[test]
fn circuit_breaker() {
    let machine: StateMachine<ClientMachineMachine> = StateMachine::new();

    // Unsuccessful request
    let machine = Arc::new(Mutex::new(machine));
    {
        let mut lock = machine.lock().unwrap();
        let res = lock.consume(&ClientMachineInput::Unsuccessful).unwrap();
        assert_eq!(res, Some(ClientMachineOutputSetTimer));
        assert_eq!(lock.state(), &ClientMachineState::Open);
    }

    // Set up a timer
    let machine_wait = machine.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::new(5, 0));
        let mut lock = machine_wait.lock().unwrap();
        let res = lock.consume(&ClientMachineInput::TimerTriggered).unwrap();
        assert_eq!(res, None);
        assert_eq!(lock.state(), &ClientMachineState::HalfOpen);
    });

    // Try to pass a request when the circuit breaker is still open
    let machine_try = machine.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::new(1, 0));
        let mut lock = machine_try.lock().unwrap();
        let res = lock.consume(&ClientMachineInput::Successful);
        assert!(matches!(res, Err(TransitionImpossibleError)));
        assert_eq!(lock.state(), &ClientMachineState::Open);
    });

    // Test if the circit breaker was actually closed
    std::thread::sleep(Duration::new(7, 0));
    {
        let mut lock = machine.lock().unwrap();
        let res = lock.consume(&ClientMachineInput::Successful).unwrap();
        assert_eq!(res, None);
        assert_eq!(lock.state(), &ClientMachineState::Closed);
    }
}
