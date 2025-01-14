use form_urlencoded::Serializer as UrlSerializer;
use ledger_apdu::APDUCommand;
use ledger_transport_hidapi::{hidapi::HidApi, TransportNativeHID};
use serde_derive::Deserialize;

use std::{env, error, process, str};

// https://github.com/LedgerHQ/ledger-live/blob/dd1d17fd3ce7ed42558204b2f93707fb9b1599de/libs/device-core/src/commands/use-cases/getVersion.ts#L6
const GET_VERSION_COMMAND: APDUCommand<&[u8]> = APDUCommand {
    cla: 0xe0,
    ins: 0x01,
    p1: 0x00,
    p2: 0x00,
    data: &[],
};

// https://github.com/LedgerHQ/ledger-live/blob/99879eb5bada1ecaea7a02d8886e16b44657af6d/libs/ledger-live-common/src/hw/listApps.ts#L5
const LIST_APPS_COMMAND: APDUCommand<&[u8]> = APDUCommand {
    cla: 0xe0,
    ins: 0xde,
    p1: 0x00,
    p2: 0x00,
    data: &[],
};

// https://github.com/LedgerHQ/ledger-live/blob/99879eb5bada1ecaea7a02d8886e16b44657af6d/libs/ledger-live-common/src/hw/listApps.ts#L47
const CONTINUE_LIST_APPS_COMMAND: APDUCommand<&[u8]> = APDUCommand {
    cla: 0xe0,
    ins: 0xdf,
    p1: 0x00,
    p2: 0x00,
    data: &[],
};

// https://github.com/LedgerHQ/ledger-live/blob/5a0a1aa5dc183116839851b79bceb6704f1de4b9/libs/ledger-live-common/src/hw/openApp.ts#L3
const OPEN_APP_COMMAND_TEMPLATE: APDUCommand<&[u8]> = APDUCommand {
    cla: 0xe0,
    ins: 0xd8,
    p1: 0x00,
    p2: 0x00,
    data: &[],
};

const LIVE_COMMON_VERSION: &str = "34.0.0";
const PROVIDER: u32 = 1; // TODO: make it possible to set it.
const BASE_API_V1_URL: &str = "https://manager.api.live.ledger.com/api";
const BASE_API_V2_URL: &str = "https://manager.api.live.ledger.com/api/v2";
const BASE_SOCKET_URL: &str = "wss://scriptrunner.api.live.ledger.com/update";

#[derive(Debug, Clone, Copy)]
enum StatusCode {
    //ACCESS_CONDITION_NOT_FULFILLED = 0x9804,
    //ALGORITHM_NOT_SUPPORTED = 0x9484,
    //CLA_NOT_SUPPORTED = 0x6e00,
    //CODE_BLOCKED = 0x9840,
    //CODE_NOT_INITIALIZED = 0x9802,
    //COMMAND_INCOMPATIBLE_FILE_STRUCTURE = 0x6981,
    //CONDITIONS_OF_USE_NOT_SATISFIED = 0x6985,
    //CONTRADICTION_INVALIDATION = 0x9810,
    //CONTRADICTION_SECRET_CODE_STATUS = 0x9808,
    //CUSTOM_IMAGE_BOOTLOADER = 0x662f,
    //CUSTOM_IMAGE_EMPTY = 0x662e,
    //FILE_ALREADY_EXISTS = 0x6a89,
    //FILE_NOT_FOUND = 0x9404,
    //GP_AUTH_FAILED = 0x6300,
    //HALTED = 0x6faa,
    //INCONSISTENT_FILE = 0x9408,
    //INCORRECT_DATA = 0x6a80,
    //INCORRECT_LENGTH = 0x6700,
    //INCORRECT_P1_P2 = 0x6b00,
    //INS_NOT_SUPPORTED = 0x6d00,
    //DEVICE_NOT_ONBOARDED = 0x6d07,
    //DEVICE_NOT_ONBOARDED_2 = 0x6611,
    //INVALID_KCV = 0x9485,
    //INVALID_OFFSET = 0x9402,
    //LICENSING = 0x6f42,
    //LOCKED_DEVICE = 0x5515,
    //MAX_VALUE_REACHED = 0x9850,
    //MEMORY_PROBLEM = 0x9240,
    //MISSING_CRITICAL_PARAMETER = 0x6800,
    //NO_EF_SELECTED = 0x9400,
    //NOT_ENOUGH_MEMORY_SPACE = 0x6a84,
    OK = 0x9000,
    //PIN_REMAINING_ATTEMPTS = 0x63c0,
    //REFERENCED_DATA_NOT_FOUND = 0x6a88,
    //SECURITY_STATUS_NOT_SATISFIED = 0x6982,
    //TECHNICAL_PROBLEM = 0x6f00,
    //UNKNOWN_APDU = 0x6d02,
    //USER_REFUSED_ON_DEVICE = 0x5501,
    //NOT_ENOUGH_SPACE = 0x5102,
}

// NOTE: MCU target id is always == target_id in Ledger Live
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct DeviceInfo {
    pub target_id: u32,
    pub version: String,
    pub flags: Vec<u8>,
    pub is_bootloader: bool,
    pub se_version: Option<String>,
    pub se_target_id: u32,
    pub mcu_version: Option<String>,
}

impl DeviceInfo {
    /// Query information about this device.
    ///
    /// Adapted from https://github.com/LedgerHQ/ledger-live/blob/dd1d17fd3ce7ed42558204b2f93707fb9b1599de/libs/device-core/src/commands/use-cases/parseGetVersionResponse.ts
    pub fn new(ledger_api: &TransportNativeHID) -> Result<Self, Box<dyn error::Error>> {
        let ver_answer = ledger_api.exchange(&GET_VERSION_COMMAND)?;
        let data = ver_answer.data();
        let mut i = 0;

        if data.len() < 5 {
            return Err("Not enough data".into());
        }
        let target_id = u32::from_be_bytes(data[i..i + 4].try_into()?);
        i += 4;
        let raw_ver_len = data[i] as usize;
        i += 1;

        if data.len() < i + raw_ver_len + 1 {
            return Err("Not enough data".into());
        }
        let raw_ver = &data[i..i + raw_ver_len];
        i += raw_ver_len;
        let version = str::from_utf8(raw_ver)?;
        let flags_len = data[i] as usize;
        i += 1;

        if data.len() < i + flags_len {
            return Err("Not enough data".into());
        }
        let flags = &data[i..i + flags_len];
        i += flags_len;

        let is_bootloader = (target_id & 4026531840) != 805306368;
        Ok(if is_bootloader {
            if data.len() < i + 1 {
                return Err("Not enough data".into());
            }
            let part1_len = data[i] as usize;
            i += 1;

            if data.len() < i + part1_len {
                return Err("Not enough data".into());
            }
            let part1 = &data[i..i + part1_len];
            i += part1_len;

            if part1_len >= 5 {
                let se_version = str::from_utf8(part1).unwrap();

                if data.len() < i + 1 {
                    return Err("Not enough data".into());
                }
                let part2_len = data[i] as usize;
                i += 1;

                if data.len() < i + part2_len {
                    return Err("Not enough data".into());
                }
                let part2 = &data[i..i + part2_len];
                //i += part2_len;
                let se_target_id = u32::from_be_bytes(part2.try_into().unwrap());

                Self {
                    target_id,
                    version: version.to_string(),
                    flags: flags.to_vec(),
                    is_bootloader,
                    se_version: Some(se_version.to_string()),
                    se_target_id,
                    mcu_version: None,
                }
            } else {
                let se_target_id = u32::from_be_bytes(part1.try_into().unwrap());

                Self {
                    target_id,
                    version: version.to_string(),
                    flags: flags.to_vec(),
                    is_bootloader,
                    se_version: None,
                    se_target_id,
                    mcu_version: None,
                }
            }
        } else {
            if data.len() < i + 1 {
                return Err("Not enough data".into());
            }
            let mcu_len = data[i] as usize;
            i += 1;

            if data.len() < i + mcu_len {
                return Err("Not enough data".into());
            }
            let mcu = &data[i..i + mcu_len];
            //i += mcu_len;
            let mcu = if mcu[mcu.len() - 1] == 0 {
                &mcu[..mcu.len() - 1]
            } else {
                mcu
            };
            let mcu_version = str::from_utf8(mcu).unwrap();

            //let osu_str = b"-osu";
            //if raw_ver.windows(osu_str.len()).any(|w| w == osu_str) {}
            //TODO. See https://github.com/LedgerHQ/ledger-live/blob/dcbda65e65ead4014e767778da6022b78d8eddad/libs/ledgerjs/packages/devices/src/index.ts#L3-L156

            Self {
                target_id,
                version: version.to_string(),
                flags: flags.to_vec(),
                is_bootloader,
                se_version: Some(version.to_string()),
                se_target_id: target_id,
                mcu_version: Some(mcu_version.to_string()),
            }
        })
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct InstalledApp {
    name: String,
    hash: Vec<u8>,
    hash_code_data: Vec<u8>,
    blocks: u16,
    flags: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum HsmMessageData {
    Command(String),
    CommandList(Vec<String>),
}

#[derive(Debug, Clone, Deserialize)]
struct HsmMessage {
    pub query: String,
    pub nonce: u32,
    pub data: Option<HsmMessageData>,
}

fn deser_apdu_command(hex_str: &str) -> Result<APDUCommand<Vec<u8>>, Box<dyn error::Error>> {
    let bytes = hex::decode(hex_str)?;
    if bytes.len() < 5 {
        return Err("Invalid command".into());
    }

    let (cla, ins, p1, p2, data_len) = (bytes[0], bytes[1], bytes[2], bytes[3], bytes[4] as usize);
    if bytes.len() != 5 + data_len {
        return Err("Invalid command".into());
    }

    Ok(APDUCommand {
        cla,
        ins,
        p1,
        p2,
        data: bytes[5..].to_vec(),
    })
}

// Some actions, such as installing apps or upgrading the firmware, are done in Ledger Live by
// opening a socket so a remote server communicates directly with the Ledger. It appears to be
// talking to an HSM up there which would manage sensitive actions.
// Parameters are passed directly in the url. Don't forget to escape the necessary characters!
fn query_via_websocket(
    ledger_api: &TransportNativeHID,
    url: &str,
) -> Result<(), Box<dyn error::Error>> {
    let (mut socket, _) = tungstenite::connect(url)?;

    // https://github.com/LedgerHQ/ledger-live/blob/99879eb5bada1ecaea7a02d8886e16b44657af6d/libs/ledger-live-common/src/socket/index.ts#L95
    loop {
        let msg = socket.read()?;
        match msg {
            // It appears they only exchange JSON text messages.
            tungstenite::Message::Text(text) => {
                let msg: HsmMessage = serde_json::from_str(&text)?;

                // The dance is usually:
                // - first the HSM sends a few standalone commands;
                // - then it sends a bunch in bulk;
                // - finally it sends a success.
                if msg.query == "exchange" {
                    let command_hex = match msg.data {
                        Some(HsmMessageData::Command(h)) => h,
                        _ => return Err("A single command is expected in 'exchange' mode.".into()),
                    };
                    let command = deser_apdu_command(&command_hex)?;

                    // NOTE: the HSM expects only the data, not the last two bytes of the raw
                    // response (the status) in the "data" field below.
                    let resp = ledger_api.exchange(&command)?;
                    let response = if resp.retcode() == StatusCode::OK as u16 {
                        "success"
                    } else {
                        eprintln!(
                            "Error when installing app. Error code: {:#02x}. Resp: {:?}.",
                            resp.retcode(),
                            resp
                        );
                        "error"
                    };
                    let resp_data = hex::encode(resp.data());

                    let ws_resp = serde_json::json!({
                        "nonce": msg.nonce,
                        "response": response,
                        "data": resp_data,
                    });
                    socket.send(tungstenite::Message::Text(serde_json::to_string(&ws_resp)?))?;
                } else if msg.query == "bulk" {
                    // Ledger Live closes the socket immediately after receiving a bulk. It doesn't
                    // appear to be necessary, on the contrary if we don't we get a clean "success"
                    // response back. So we might as well do that.
                    //socket.close(None).unwrap();

                    let commands = match msg.data {
                        Some(HsmMessageData::CommandList(l)) => l,
                        _ => return Err("Expecting a list of commands in bulk mode.".into()),
                    };
                    for cmd_hex in commands {
                        if cmd_hex.is_empty() {
                            continue;
                        }
                        let command = deser_apdu_command(&cmd_hex)?;
                        let _ = ledger_api.exchange(&command)?;
                    }

                    let ws_resp = serde_json::json!({
                        "nonce": msg.nonce,
                        "response": "success",
                        "data": "",
                    });
                    socket.send(tungstenite::Message::Text(serde_json::to_string(&ws_resp)?))?;
                } else if msg.query == "success" {
                    return Ok(());
                } else if msg.query == "error" {
                    return Err(
                        format!("Got an 'error' query on the ws. Full message: {}.", text).into(),
                    );
                } else if msg.query == "warning" {
                    eprintln!("Got a 'warning' query on the ws. Full message: {}.", text);
                } else {
                    return Err(format!(
                        "Got an unsupported query on the ws. Full message: {}.",
                        text
                    )
                    .into());
                }
            }
            _ => {
                return Err(format!(
                    "Got an unsupported message type on the ws. Message: {:?}.",
                    msg
                )
                .into())
            }
        }
    }
}

// Print on stderr and exit with 1.
macro_rules! error {
    ($($arg:tt)*) => {{
        eprintln!($($arg)*);
        process::exit(1);
    }};
}

#[derive(Debug, Clone, Copy)]
enum Command {
    GetInfo,
    GenuineCheck,
    InstallMainApp,
    UpdateMainApp,
    OpenMainApp,
    InstallTestApp,
    UpdateTestApp,
    OpenTestApp,
    UpdateeFirmware,
}

impl Command {
    /// Read command from environment variables.
    pub fn get() -> Option<Self> {
        let is_testnet = env::var("LEDGER_TESTNET").is_ok();
        let cmd_str = env::var("LEDGER_COMMAND").ok()?;

        if cmd_str == "getinfo" {
            Some(Self::GetInfo)
        } else if cmd_str == "genuinecheck" {
            Some(Self::GenuineCheck)
        } else if cmd_str == "installapp" {
            Some(if is_testnet {
                Self::InstallTestApp
            } else {
                Self::InstallMainApp
            })
        } else if cmd_str == "updateapp" {
            Some(if is_testnet {
                Self::UpdateTestApp
            } else {
                Self::UpdateMainApp
            })
        } else if cmd_str == "openapp" {
            Some(if is_testnet {
                Self::OpenTestApp
            } else {
                Self::OpenMainApp
            })
        } else if cmd_str == "updatefirm" {
            Some(Self::UpdateeFirmware)
        } else {
            None
        }
    }
}

fn ledger_api() -> TransportNativeHID {
    let hid_api = match HidApi::new() {
        Ok(a) => a,
        Err(e) => error!("Error initializing HDI api: {}.", e),
    };
    match TransportNativeHID::new(&hid_api) {
        Ok(a) => a,
        Err(e) => error!("Error connecting to Ledger device: {}.", e),
    }
}

fn device_info(ledger_api: &TransportNativeHID) -> DeviceInfo {
    match DeviceInfo::new(&ledger_api) {
        Ok(i) => i,
        Err(e) => error!("Error fetching device info: {}. Is the Ledger unlocked?", e),
    }
}

fn list_installed_apps(
    ledger_api: &TransportNativeHID,
) -> Result<Vec<InstalledApp>, Box<dyn error::Error>> {
    println!("Querying installed applications from your Ledger. You might have to confirm on your device.");

    let mut answer = ledger_api.exchange(&LIST_APPS_COMMAND)?;
    let mut data = answer.data();

    // See https://github.com/LedgerHQ/ledger-live/blob/99879eb5bada1ecaea7a02d8886e16b44657af6d/libs/ledger-live-common/src/hw/listApps.ts#L9
    let mut installed_apps = Vec::new();
    while !data.is_empty() {
        let mut i = 0;
        assert_eq!(data[i], 0x01);
        i += 1;

        while i < data.len() {
            if data.len() < i + 1 + 2 + 2 + 32 + 32 + 1 {
                return Err("Not enough data".into());
            }

            let len = data[i] as usize;
            i += 1;
            let blocks = u16::from_be_bytes(data[i..i + 2].try_into()?);
            i += 2;
            let flags = u16::from_be_bytes(data[i..i + 2].try_into()?);
            i += 2;
            let hash_code_data = data[i..i + 32].to_vec();
            i += 32;
            let hash = data[i..i + 32].to_vec();
            i += 32;
            let name_len = data[i] as usize;
            i += 1;

            if data.len() < i + name_len {
                return Err("Not enough data".into());
            }
            if len != name_len + 70 {
                return Err("Invalid listApps length data.".into());
            }
            let name = str::from_utf8(&data[i..i + name_len])?.to_string();
            i += name_len;

            installed_apps.push(InstalledApp {
                name,
                hash,
                hash_code_data,
                blocks,
                flags,
            });
        }

        answer = ledger_api.exchange(&CONTINUE_LIST_APPS_COMMAND)?;
        data = answer.data();
    }

    Ok(installed_apps)
}

fn print_ledger_info(ledger_api: &TransportNativeHID) {
    let device_info = device_info(ledger_api);
    println!("Information about the device: {:#?}", device_info);

    let apps = match list_installed_apps(&ledger_api) {
        Ok(a) => a,
        Err(e) => error!("Error listing installed applications: {}.", e),
    };
    println!("Installed applications:");
    for app in apps {
        println!("  - {:?}", app);
    }
}

#[derive(Debug, Clone, Deserialize)]
struct DeviceVersion {
    pub id: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct FirmwareInfo {
    pub perso: String,
}

impl FirmwareInfo {
    pub fn from_device(device_info: &DeviceInfo) -> Self {
        let dev_ver_resp = minreq::Request::new(
            minreq::Method::Post,
            &format!("{}/get_device_version", BASE_API_V1_URL),
        )
        .with_param("livecommonversion", LIVE_COMMON_VERSION)
        .with_json(&serde_json::json!({
        "provider": PROVIDER,
        "target_id": device_info.target_id,
        }))
        .unwrap()
        .send()
        .unwrap();
        let device_version = dev_ver_resp.json::<DeviceVersion>().unwrap();

        let firm_resp = minreq::Request::new(
            minreq::Method::Post,
            &format!("{}/get_firmware_version", BASE_API_V1_URL),
        )
        .with_param("livecommonversion", LIVE_COMMON_VERSION)
        .with_json(&serde_json::json!({
        "provider": PROVIDER,
        "device_version": device_version.id,
        "version_name": &device_info.version,
        }))
        .unwrap()
        .send()
        .unwrap();
        firm_resp.json::<FirmwareInfo>().unwrap()
    }
}

fn genuine_check(ledger_api: &TransportNativeHID) {
    let device_info = device_info(ledger_api);
    let firmware_info = FirmwareInfo::from_device(&device_info);

    println!("Querying Ledger's remote HSM to perform the genuine check. You might have to confirm the operation on your device.");
    let genuine_ws_url = UrlSerializer::new(format!("{}/genuine?", BASE_SOCKET_URL))
        .append_pair("targetId", &device_info.target_id.to_string())
        .append_pair("perso", &firmware_info.perso)
        .finish();
    if let Err(e) = query_via_websocket(&ledger_api, &genuine_ws_url) {
        error!("Error when performing genuine check: {}.", e);
    }
    println!("Success. Your Ledger is genuine.");
}

// DON'T DELETE ME JUST YET.
// This appears to be the "old" (api v1) way of querying information about the Bitcoin app for a
// device. It does give access to more data, so i'm keeping it around for now just in case.
//
// See
// https://github.com/LedgerHQ/ledger-live/blob/99879eb5bada1ecaea7a02d8886e16b44657af6d/libs/ledger-live-common/src/manager/index.ts#L103-L104.
//
// See above for firmware info.
//
//let compatible_apps = minreq::Request::new(
//minreq::Method::Post,
//"https://manager.api.live.ledger.com/api/get_apps",
//)
//.with_param("livecommonversion", "34.0.0")
//.with_json(&serde_json::json!({
//"provider": PROVIDER,
//"current_se_firmware_final_version": firmware_id,
//"device_version": device_id,
//}))
//.unwrap()
//.with_param("firmware_version_name", device_info.version)
//.send()
//.unwrap();
//let bitcoin_apps: Vec<_> = compatible_apps
//.json::<serde_json::Value>()
//.unwrap()
//.get("application_versions")
//.unwrap()
//.as_array()
//.unwrap()
//.into_iter()
//.filter(|o| {
//o.as_object()
//.unwrap()
//.get("name")
//.unwrap()
//.as_str()
//.unwrap()
//.to_lowercase()
//== "bitcoin test"
////.contains("bitcoin")
//})
////.inspect(|o| println!("{}", serde_json::to_string_pretty(&o).unwrap()))
//.cloned()
//.collect();
//let bitcoin_app = &bitcoin_apps[0];
//println!("{}", bitcoin_app);

#[derive(Debug, Clone, Deserialize)]
struct BitcoinAppV2 {
    #[serde(rename = "versionName")]
    version_name: String,
    perso: String,
    #[serde(rename = "deleteKey")]
    delete_key: String,
    firmware: String,
    #[serde(rename = "firmwareKey")]
    firmware_key: String,
    hash: String,
}

// Install the Bitcoin app on the device.
fn install_app(ledger_api: &TransportNativeHID, is_testnet: bool) {
    // First of all make sure it's not already installed.
    let lowercase_app_name = if is_testnet {
        "bitcoin test"
    } else {
        "bitcoin"
    };
    let apps = match list_installed_apps(&ledger_api) {
        Ok(a) => a,
        Err(e) => error!("Error listing installed applications: {}.", e),
    };
    if apps
        .iter()
        .any(|app| app.name.to_lowercase() == lowercase_app_name)
    {
        //error!("Bitcoin app already installed. Use the update command to update it.");
    }

    // Get the Bitcoin app information for this device.
    // This uses the v2 API. See for reference:
    // - https://github.com/LedgerHQ/ledger-live/blob/5a0a1aa5dc183116839851b79bceb6704f1de4b9/libs/ledger-live-common/src/apps/listApps/v2.ts
    // - https://github.com/LedgerHQ/ledger-live/blob/5a0a1aa5dc183116839851b79bceb6704f1de4b9/libs/device-core/src/managerApi/repositories/HttpManagerApiRepository.ts#L211
    // There is also another way which seems to be the API v1 way of getting the app info. See
    // above the commented out code.
    let device_info = device_info(ledger_api);
    let resp_apps = minreq::Request::new(
        minreq::Method::Get,
        format!("{}/apps/by-target", BASE_API_V2_URL),
    )
    .with_param("livecommonversion", "34.0.0")
    .with_param("provider", PROVIDER.to_string()) // TODO: allow to configure the provider
    .with_param("target_id", device_info.target_id.to_string())
    .with_param("firmware_version_name", device_info.version.clone())
    .send();
    let resp_apps = match resp_apps {
        Ok(r) => r,
        Err(e) => error!(
            "Error when querying information about the Bitcoin app for this device: {}.",
            e
        ),
    };
    let bitcoin_app = resp_apps
        .json::<Vec<BitcoinAppV2>>()
        // FIXME: is versionName guaranteed to be the name? What's "version" for?
        .map(|apps| {
            apps.into_iter()
                .find(|o| o.version_name.to_lowercase() == lowercase_app_name)
        });
    let bitcoin_app = match bitcoin_app {
        Ok(Some(a)) => a,
        Ok(None) => {
            error!("Could not get info about Bitcoin app.",);
        }
        Err(e) => {
            error!(
                "Error when deserializing response into list of Bitcoin apps info: {}",
                e
            );
        }
    };

    // Now install the app by connecting through their websocket thing to their HSM. Make sure to
    // properly escape the parameters in the request's parameter.
    let install_ws_url = UrlSerializer::new(format!("{}/install?", BASE_SOCKET_URL))
        .append_pair("targetId", &device_info.target_id.to_string())
        .append_pair("perso", &bitcoin_app.perso)
        .append_pair("deleteKey", &bitcoin_app.delete_key)
        .append_pair("firmware", &bitcoin_app.firmware)
        .append_pair("firmwareKey", &bitcoin_app.firmware_key)
        .append_pair("hash", &bitcoin_app.hash)
        .finish();
    println!("Querying Ledger's remote HSM to install the app. You might have to confirm the operation on your device.");
    if let Err(e) = query_via_websocket(&ledger_api, &install_ws_url) {
        error!(
            "Got an error when installing Bitcoin app from Ledger's remote HSM: {}.",
            e
        );
    }
    println!("Successfully installed the app.");
}

fn open_app(ledger_api: &TransportNativeHID, name: &[u8]) {
    let mut command = OPEN_APP_COMMAND_TEMPLATE;
    command.data = name;

    println!("Opening app on your Ledger. You might have to confirm on your device.",);
    let resp = match ledger_api.exchange(&command) {
        Ok(r) => r,
        Err(e) => error!("Error opening app: {}.", e),
    };
    if resp.retcode() != StatusCode::OK as u16 {
        error!("Error opening app. Ledger response: {:#x?}.", resp);
    }
}

fn main() {
    let command = if let Some(cmd) = Command::get() {
        cmd
    } else {
        error!("Invalid or no command specified. The command must be passed through the LEDGER_COMMAND env var. Set LEDGER_TESTNET to use the Bitcoin testnet app instead where applicable.");
    };

    let ledger_api = ledger_api();
    match command {
        Command::GetInfo => {
            print_ledger_info(&ledger_api);
        }
        Command::GenuineCheck => {
            genuine_check(&ledger_api);
        }
        Command::InstallMainApp => {
            install_app(&ledger_api, false);
        }
        Command::InstallTestApp => {
            install_app(&ledger_api, true);
        }
        Command::OpenMainApp => {
            open_app(&ledger_api, b"Bitcoin");
        }
        Command::OpenTestApp => {
            open_app(&ledger_api, b"Bitcoin Test");
        }
        Command::UpdateMainApp | Command::UpdateTestApp | Command::UpdateeFirmware => {
            unimplemented!()
        }
    }
}
