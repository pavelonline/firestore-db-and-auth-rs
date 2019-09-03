#![cfg_attr(feature = "external_doc", feature(external_doc))]
#![cfg_attr(feature = "external_doc", doc(include = "../readme.md"))]

extern crate regex;
extern crate ring;
extern crate untrusted;

pub mod credentials;
pub mod documents;
pub mod errors;
pub mod firebase_rest_to_rust;
pub mod sessions;
pub mod users;

mod dto;

#[cfg(feature = "rocket_support")]
pub mod rocket;

// Forward declarations
pub use credentials::Credentials;

/// Authentication trait.
///
/// This trait is implemented by [`crate::sessions`], but you do not need those for using the Firestore API of this crate.
/// Firestore document methods in [`crate::documents`] expect an object that implements the `FirebaseAuthBearer` trait.
///
/// Implement this trait for your own data structure and provide the Firestore project id and a valid access token.
pub trait FirebaseAuthBearer<'a> {
    /// Return the project ID. This is required for the firebase REST API.
    fn projectid(&'a self) -> &'a str;
    /// An access token, preferably valid.
    fn bearer(&'a self) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[test]
    fn service_account_session() -> errors::Result<()> {
        let cred = credentials::Credentials::from_file("firebase-service-account.json")
            .expect("Read credentials file");

        assert!(cred.public_key(&cred.private_key_id).is_some());
        let mut session = sessions::service_account::Session::new(cred).unwrap();
        let b = session.bearer().to_owned();

        // Check if cached value is used
        assert_eq!(session.bearer(), b);

        let obj = DemoDTO {
            a_string: "abcd".to_owned(),
            an_int: 14,
            a_timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        };

        documents::write(&mut session, "tests", Some("service_test"), &obj)?;

        let read: DemoDTO = documents::read(&mut session, "tests", "service_test")?;

        assert_eq!(read.a_string, "abcd");
        assert_eq!(read.an_int, 14);

        Ok(())
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct DemoDTO {
        a_string: String,
        an_int: u32,
        a_timestamp: String,
    }

    #[test]
    fn user_account_session() -> errors::Result<()> {
        let test_user_id = "Io2cPph06rUWM3ABcIHguR3CIw6v1";

        let cred = credentials::Credentials::from_file("firebase-service-account.json")
            .expect("Read credentials file");

        // Read refresh token from file if possible instead of generating a new refresh token each time
        let refresh_token: String = match std::fs::read_to_string("refresh-token-for-tests.txt") {
            Ok(v) => v,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    return Err(errors::FirebaseError::IO(e));
                }
                String::new()
            }
        };

        // Generate a new refresh token if necessary
        let user_session: sessions::user::Session = if refresh_token.is_empty() {
            let session = sessions::user::Session::by_user_id(&cred, test_user_id)?;
            std::fs::write(
                "refresh-token-for-tests.txt",
                &session.refresh_token.as_ref().unwrap(),
            )?;
            session
        } else {
            sessions::user::Session::by_refresh_token(&cred, &refresh_token)?
        };

        assert_eq!(user_session.userid, test_user_id);
        assert_eq!(user_session.projectid, cred.project_id);

        let mut user_session =
            sessions::user::Session::by_access_token(&cred, &user_session.bearer)?;

        assert_eq!(user_session.userid, test_user_id);

        let obj = DemoDTO {
            a_string: "abc".to_owned(),
            an_int: 12,
            a_timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        };

        // Test writing
        let result = documents::write(&mut user_session, "tests", Some("test"), &obj)?;
        assert_eq!(result.document_id, "test");
        let duration = chrono::Utc::now().signed_duration_since(result.update_time.unwrap());
        assert!(
            duration.num_seconds() < 60,
            "now = {}, updated: {}, created: {}",
            chrono::Utc::now(),
            result.update_time.unwrap(),
            result.create_time.unwrap()
        );

        // Test reading
        let read: DemoDTO = documents::read(&mut user_session, "tests", "test")?;

        assert_eq!(read.a_string, "abc");
        assert_eq!(read.an_int, 12);

        let user_info_container = users::userinfo(&user_session)?;
        assert_eq!(
            user_info_container.users[0].localId.as_ref().unwrap(),
            test_user_id
        );

        // Query for all documents with field "a_string" and value "abc"
        let results: Vec<DemoDTO> = documents::query(
            &mut user_session,
            "tests",
            "abc",
            dto::FieldOperator::EQUAL,
            "a_string",
        )?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].a_string, "abc");

        let mut count = 0;
        let list_it: documents::List<DemoDTO, _> = documents::list(&mut user_session, "tests");
        for _doc in list_it {
            count += 1;
        }
        assert_eq!(count, 2);

        // test if the call fails for a non existing document
        let r = documents::delete(&mut user_session, "tests/non_existing", true);
        assert!(r.is_err());

        documents::delete(&mut user_session, "tests/test", false)?;

        // Check if document is indeed removed
        let results: Vec<DemoDTO> = documents::query(
            &mut user_session,
            "tests",
            "abc",
            dto::FieldOperator::EQUAL,
            "a_string",
        )?;
        assert_eq!(results.len(), 0);

        Ok(())
    }
}
