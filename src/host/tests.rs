use super::*;

struct Says(Option<&'static str>);

impl Host for Says {
    fn caller(&self, _: &Env) -> Result<Option<Caller>, Failure> {
        Ok(self.0.map(|s| Caller {
            session_id: Some(s.into()),
            ..Caller::default()
        }))
    }
}

fn who(hosts: Vec<Box<dyn Host>>) -> Option<String> {
    caller(&hosts, &Env::default()).unwrap().session_id
}

#[test]
fn the_first_host_that_holds_the_caller_answers() {
    assert_eq!(
        who(vec![Box::new(Says(None)), Box::new(Says(Some("b")))]),
        Some("b".into())
    );
    assert_eq!(
        who(vec![Box::new(Says(Some("a"))), Box::new(Says(Some("b")))]),
        Some("a".into())
    );
}

#[test]
fn with_no_host_the_caller_has_no_seat() {
    assert_eq!(who(Vec::new()), None);
    let c = caller(&[], &Env::default()).unwrap();
    assert_eq!(c, Caller::default());
}
