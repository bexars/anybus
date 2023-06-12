use uuid::Uuid;

pub fn random_uuid() -> Uuid {
    use rand::prelude::*;
    let random_bytes = thread_rng().gen();

    let uuid = uuid::Builder::from_random_bytes(random_bytes).into_uuid();

    uuid
}
