use uuid::Uuid;

pub fn random_uuid() -> Uuid {
    // use rand::prelude::*;
    let random_bytes = rand::random();
    
    // rng().random();

    let uuid = uuid::Builder::from_random_bytes(random_bytes).into_uuid();

    uuid
}
