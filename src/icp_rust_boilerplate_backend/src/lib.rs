#[macro_use]
extern crate serde;
use candid::{Decode, Encode};
use ic_cdk::api::time;
use ic_stable_structures::memory_manager::{MemoryId, MemoryManager, VirtualMemory};
use ic_stable_structures::{BoundedStorable, Cell, DefaultMemoryImpl, StableBTreeMap, Storable};
use std::{borrow::Cow, cell::RefCell};

type Memory = VirtualMemory<DefaultMemoryImpl>;
type IdCell = Cell<u64, Memory>;

#[derive(candid::CandidType, Clone, Serialize, Deserialize, Default)]
struct Book {
    id: u64,
    title: String,
    author: String,
    available: bool,
    created_at: u64,
    updated_at: Option<u64>,
}

#[derive(candid::CandidType, Clone, Serialize, Deserialize, Default)]
struct Rental {
    id: u64,
    book_id: u64,
    user_id: String,
    rented_at: u64,
    due_date: u64,
    returned_at: Option<u64>,
}

// Trait implementations for Book
impl Storable for Book {
    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }
}

impl BoundedStorable for Book {
    const MAX_SIZE: u32 = 512;
    const IS_FIXED_SIZE: bool = false;
}

// Trait implementations for Rental
impl Storable for Rental {
    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }
}

impl BoundedStorable for Rental {
    const MAX_SIZE: u32 = 512;
    const IS_FIXED_SIZE: bool = false;
}

thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> = RefCell::new(
        MemoryManager::init(DefaultMemoryImpl::default())
    );

    static BOOK_ID_COUNTER: RefCell<IdCell> = RefCell::new(
        IdCell::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0))), 0)
            .expect("Cannot create a counter")
    );

    static RENTAL_ID_COUNTER: RefCell<IdCell> = RefCell::new(
        IdCell::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(1))), 0)
            .expect("Cannot create a counter")
    );

    static BOOK_STORAGE: RefCell<StableBTreeMap<u64, Book, Memory>> =
        RefCell::new(StableBTreeMap::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(2)))
    ));

    static RENTAL_STORAGE: RefCell<StableBTreeMap<u64, Rental, Memory>> =
        RefCell::new(StableBTreeMap::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(3)))
    ));
}

#[derive(candid::CandidType, Serialize, Deserialize, Default)]
struct BookPayload {
    title: String,
    author: String,
}

#[ic_cdk::query]
fn get_book(id: u64) -> Result<Book, Error> {
    match _get_book(&id) {
        Some(book) => Ok(book),
        None => Err(Error::NotFound {
            msg: format!("A book with id={} not found", id),
        }),
    }
}

#[ic_cdk::update]
fn add_book(book: BookPayload) -> Option<Book> {
    let id = BOOK_ID_COUNTER
        .with(|counter| {
            let current_value = *counter.borrow().get();
            counter.borrow_mut().set(current_value + 1)
        })
        .expect("Cannot increment book id counter");
    let book = Book {
        id,
        title: book.title,
        author: book.author,
        available: true,
        created_at: time(),
        updated_at: None,
    };
    do_insert_book(&book);
    Some(book)
}

#[ic_cdk::update]
fn update_book(id: u64, payload: BookPayload) -> Result<Book, Error> {
    match BOOK_STORAGE.with(|service| service.borrow().get(&id)) {
        Some(mut book) => {
            book.title = payload.title;
            book.author = payload.author;
            book.updated_at = Some(time());
            do_insert_book(&book);
            Ok(book)
        }
        None => Err(Error::NotFound {
            msg: format!(
                "Couldn't update a book with id={}. Book not found",
                id
            ),
        }),
    }
}

#[ic_cdk::update]
fn delete_book(id: u64) -> Result<Book, Error> {
    match BOOK_STORAGE.with(|service| service.borrow_mut().remove(&id)) {
        Some(book) => Ok(book),
        None => Err(Error::NotFound {
            msg: format!(
                "Couldn't delete a book with id={}. Book not found.",
                id
            ),
        }),
    }
}

#[ic_cdk::update]
fn rent_book(book_id: u64, user_id: String, due_date: u64) -> Result<Rental, Error> {
    match BOOK_STORAGE.with(|service| service.borrow().get(&book_id)) {
        Some(mut book) => {
            if !book.available {
                return Err(Error::BookUnavailable {
                    msg: format!("The book with id={} is not available for rent", book_id),
                });
            }
            book.available = false;
            book.updated_at = Some(time());
            do_insert_book(&book);

            let rental_id = RENTAL_ID_COUNTER
                .with(|counter| {
                    let current_value = *counter.borrow().get();
                    counter.borrow_mut().set(current_value + 1)
                })
                .expect("Cannot increment rental id counter");
            let rental = Rental {
                id: rental_id,
                book_id,
                user_id,
                rented_at: time(),
                due_date,
                returned_at: None,
            };
            do_insert_rental(&rental);
            Ok(rental)
        }
        None => Err(Error::NotFound {
            msg: format!("A book with id={} not found", book_id),
        }),
    }
}

#[ic_cdk::update]
fn return_book(rental_id: u64) -> Result<Rental, Error> {
    match RENTAL_STORAGE.with(|service| service.borrow().get(&rental_id)) {
        Some(mut rental) => {
            if rental.returned_at.is_some() {
                return Err(Error::AlreadyReturned {
                    msg: format!("The rental with id={} has already been returned", rental_id),
                });
            }
            rental.returned_at = Some(time());

            if let Some(mut book) = BOOK_STORAGE.with(|service| service.borrow().get(&rental.book_id)) {
                book.available = true;
                book.updated_at = Some(time());
                do_insert_book(&book);
            }

            do_insert_rental(&rental);
            Ok(rental)
        }
        None => Err(Error::NotFound {
            msg: format!("A rental with id={} not found", rental_id),
        }),
    }
}

// Helper methods for insertions
fn do_insert_book(book: &Book) {
    BOOK_STORAGE.with(|service| service.borrow_mut().insert(book.id, book.clone()));
}

fn do_insert_rental(rental: &Rental) {
    RENTAL_STORAGE.with(|service| service.borrow_mut().insert(rental.id, rental.clone()));
}

#[derive(candid::CandidType, Deserialize, Serialize)]
enum Error {
    NotFound { msg: String },
    BookUnavailable { msg: String },
    AlreadyReturned { msg: String },
}

fn _get_book(id: &u64) -> Option<Book> {
    BOOK_STORAGE.with(|service| service.borrow().get(id))
}

// Generate candid interface
ic_cdk::export_candid!();
