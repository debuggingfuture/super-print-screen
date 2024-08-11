// Runs a simple Prover which connects to the Notary and notarizes a request/response from
// example.com. The Prover then generates a proof and writes it to disk.
mod openai;

use std::env;

use http_body_util::Empty;
use hyper::{body::Bytes, Request, StatusCode};
use hyper_util::rt::TokioIo;
use std::ops::Range;
use tlsn_core::proof::TlsProof;
use ethers::abi::Token;
use tokio::io::AsyncWriteExt as _;
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};
use eas_sdk_rs::eas::AttestationDataBuilder;
use tlsn_prover::tls::{state::Notarize, Prover, ProverConfig};

use ethers::core::k256::ecdsa::SigningKey;
use ethers::prelude::*;
use ethers::utils::hex;

use std::str::FromStr;

// Setting of the application server
const SERVER_DOMAIN: &str = "api.openai.com";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/114.0.0.0 Safari/537.36";

use std::str;

use elliptic_curve::pkcs8::DecodePrivateKey;
use futures::{AsyncRead, AsyncWrite};
use tlsn_verifier::tls::{Verifier, VerifierConfig};


use openai::{Message, OpenAIRequest};

const BASE_SEPOLIA_CONTRACT_ADDRESS: &str = "0x4200000000000000000000000000000000000021";
const BASE_SEPOLIA_ENDPOINT: &str = "https://sepolia.base.org/";


pub async fn create_attestation(proof: String) -> Result<(), Box<dyn std::error::Error>> {
    let private_key_hex = env::var("PRIVATE_KEY_HEX").expect("PRIVATE_KEY_HEX must be set");

    let signing_key = SigningKey::from_slice(&hex::decode(private_key_hex)?)?;
    let contract_address = H160::from_str(BASE_SEPOLIA_CONTRACT_ADDRESS)?;
    let client = eas_sdk_rs::client::Client::new(BASE_SEPOLIA_ENDPOINT, contract_address, None, signing_key).await?;

    let schema_uid = H256::from_str("0x937f07b2538a23865e59d6f2a9a109b0e2da3dad79c8b6702b80cb15ebd8a9a7")?;

    // Make a new attestation for the created schema
    let attestation_request = AttestationDataBuilder::new()
        .revocable(true)
        .data(&[
            Token::String(proof)
        ])
        .build();

    // attest sends a transaction and returns a PendingTx that can be awaited
    let tx = client.eas.attest(&schema_uid, attestation_request).await?;
    let attested = tx.await?;

    println!("Attestation ID: {:?}", attested);

    Ok(())

}

/// Runs a simple Notary with the provided connection to the Prover.
pub async fn run_notary<T: AsyncWrite + AsyncRead + Send + Unpin + 'static>(conn: T) {
    // Load the notary signing key
    let signing_key_str = std::str::from_utf8(include_bytes!(
        "../fixture/notary/notary.key"
    ))
    .unwrap();
    let signing_key = p256::ecdsa::SigningKey::from_pkcs8_pem(signing_key_str).unwrap();

    // Setup default config. Normally a different ID would be generated
    // for each notarization.
    let config = VerifierConfig::builder().id("example").build().unwrap();

    Verifier::new(config)
        .notarize::<_, p256::ecdsa::Signature>(conn, &signing_key)
        .await
        .unwrap();
}

fn create_openai_request() -> OpenAIRequest {
    OpenAIRequest {
        model: "gpt-4o-mini".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: "Who is the current president of USA?".to_string(),
        }],
        temperature: 0.7,
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();


    let api_key = env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");

    openai::create_thread().await.unwrap();

    println!("create thread done");

    let (prover_socket, notary_socket) = tokio::io::duplex(1 << 16);

    // Start a local simple notary service
    tokio::spawn(run_notary(notary_socket.compat()));

    // A Prover configuration
    let config = ProverConfig::builder()
        .id("example")
        .server_dns(SERVER_DOMAIN)
        .build()
        .unwrap();

    // Create a Prover and set it up with the Notary
    // This will set up the MPC backend prior to connecting to the server.
    let prover = Prover::new(config)
        .setup(prover_socket.compat())
        .await
        .unwrap();

    // Connect to the Server via TCP. This is the TLS client socket.
    let client_socket = tokio::net::TcpStream::connect((SERVER_DOMAIN, 443))
        .await
        .unwrap();

    // Bind the Prover to the server connection.
    // The returned `mpc_tls_connection` is an MPC TLS connection to the Server: all data written
    // to/read from it will be encrypted/decrypted using MPC with the Notary.
    let (mpc_tls_connection, prover_fut) = prover.connect(client_socket.compat()).await.unwrap();
    let mpc_tls_connection = TokioIo::new(mpc_tls_connection.compat());

    // Spawn the Prover task to be run concurrently
    let prover_task = tokio::spawn(prover_fut);

    // Attach the hyper HTTP client to the MPC TLS connection
    let (mut request_sender, connection) =
        hyper::client::conn::http1::handshake(mpc_tls_connection)
            .await
            .unwrap();

    // Spawn the HTTP task to be run concurrently
    tokio::spawn(connection);

    let thread_id = "thread_JApUi9tQ7ucc22fyrA0lEK5z";

    let message_id = "msg_thRMhxU4wgoOl2M2jJBsVx4V";

    println!("/v1/threads/{}/messages/{}", thread_id, message_id);

    // Build a simple HTTP request with common headers
    let request = Request::builder()
        .uri(format!("/v1/threads/{}/messages", thread_id))
        .header("Host", SERVER_DOMAIN)
        .header("Accept", "*/*")
        // Using "identity" instructs the Server not to use compression for its HTTP response.
        // TLSNotary tooling does not support compression.
        .header("Accept-Encoding", "identity")
        .header("Connection", "close")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("OpenAI-Beta","assistants=v2")
        .header("User-Agent", USER_AGENT)
        .header("Content-Type", "application/json")
        .body(Empty::<Bytes>::new())
        .unwrap();

    println!("Starting an MPC TLS connection with the server");

    // Send the request to the Server and get a response via the MPC TLS connection
    let response = request_sender.send_request(request).await.unwrap();

    println!("Got a response from the server");

    assert!(response.status() == StatusCode::OK);

    // The Prover task should be done now, so we can grab the Prover.
    let prover = prover_task.await.unwrap().unwrap();

    // Prepare for notarization.
    let prover = prover.start_notarize();

    // Build proof (with or without redactions)
    let redact = false;
    let proof = if !redact {
        build_proof_without_redactions(prover).await
    } else {
        build_proof_with_redactions(prover).await
    };

    let proof_string = serde_json::to_string(&proof).unwrap();

    // Write the proof to a file
    let mut file = tokio::fs::File::create("simple_proof.json").await.unwrap();
    file.write_all(proof_string.clone().as_bytes())
        .await
        .unwrap();

    println!("Notarization completed successfully!");
    println!("The proof has been written to `simple_proof.json`");

    // dont use the pretty one

    // alternatively, upload proof ipfs

    create_attestation(proof_string).await.unwrap();

    println!("created attestation completed");

}

/// Find the ranges of the public and private parts of a sequence.
///
/// Returns a tuple of `(public, private)` ranges.
fn find_ranges(seq: &[u8], private_seq: &[&[u8]]) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let mut private_ranges = Vec::new();
    for s in private_seq {
        for (idx, w) in seq.windows(s.len()).enumerate() {
            if w == *s {
                private_ranges.push(idx..(idx + w.len()));
            }
        }
    }

    let mut sorted_ranges = private_ranges.clone();
    sorted_ranges.sort_by_key(|r| r.start);

    let mut public_ranges = Vec::new();
    let mut last_end = 0;
    for r in sorted_ranges {
        if r.start > last_end {
            public_ranges.push(last_end..r.start);
        }
        last_end = r.end;
    }

    if last_end < seq.len() {
        public_ranges.push(last_end..seq.len());
    }

    (public_ranges, private_ranges)
}

async fn build_proof_without_redactions(mut prover: Prover<Notarize>) -> TlsProof {
    let sent_len = prover.sent_transcript().data().len();
    let recv_len = prover.recv_transcript().data().len();

    let builder = prover.commitment_builder();
    let sent_commitment = builder.commit_sent(&(0..sent_len)).unwrap();
    let recv_commitment = builder.commit_recv(&(0..recv_len)).unwrap();

    // Finalize, returning the notarized session
    let notarized_session = prover.finalize().await.unwrap();

    // Create a proof for all committed data in this session
    let mut proof_builder = notarized_session.data().build_substrings_proof();

    // Reveal all the public ranges
    proof_builder.reveal_by_id(sent_commitment).unwrap();
    proof_builder.reveal_by_id(recv_commitment).unwrap();

    let substrings_proof = proof_builder.build().unwrap();

    TlsProof {
        session: notarized_session.session_proof(),
        substrings: substrings_proof,
    }
}

async fn build_proof_with_redactions(mut prover: Prover<Notarize>) -> TlsProof {
    // Identify the ranges in the outbound data which contain data which we want to disclose
    let (sent_public_ranges, _) = find_ranges(
        prover.sent_transcript().data(),
        &[
            // Redact the value of the "User-Agent" header. It will NOT be disclosed.
            USER_AGENT.as_bytes(),
        ],
    );

    // Identify the ranges in the inbound data which contain data which we want to disclose
    let (recv_public_ranges, _) = find_ranges(
        prover.recv_transcript().data(),
        &[
            // Redact the value of the title. It will NOT be disclosed.
            "Example Domain".as_bytes(),
        ],
    );

    let builder = prover.commitment_builder();

    // Commit to each range of the public outbound data which we want to disclose
    let sent_commitments: Vec<_> = sent_public_ranges
        .iter()
        .map(|range| builder.commit_sent(range).unwrap())
        .collect();
    // Commit to each range of the public inbound data which we want to disclose
    let recv_commitments: Vec<_> = recv_public_ranges
        .iter()
        .map(|range| builder.commit_recv(range).unwrap())
        .collect();

    // Finalize, returning the notarized session
    let notarized_session = prover.finalize().await.unwrap();

    // Create a proof for all committed data in this session
    let mut proof_builder = notarized_session.data().build_substrings_proof();

    // Reveal all the public ranges
    for commitment_id in sent_commitments {
        proof_builder.reveal_by_id(commitment_id).unwrap();
    }
    for commitment_id in recv_commitments {
        proof_builder.reveal_by_id(commitment_id).unwrap();
    }

    let substrings_proof = proof_builder.build().unwrap();

    TlsProof {
        session: notarized_session.session_proof(),
        substrings: substrings_proof,
    }
}
