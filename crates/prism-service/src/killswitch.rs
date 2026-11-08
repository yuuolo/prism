//! Kill switch: Windows Filtering Platform filters that stop connections
//! from leaving outside the TUN adapter, so a core crash can't turn TUN
//! traffic into direct traffic.
//!
//! The filters live in a dynamic WFP session held by the service. They stay
//! when the core dies and vanish when the session ends: a stopped or crashed
//! service, or a reboot, can never leave the machine cut off.
//!
//! Still allowed: the TUN adapter's addresses, loopback, the given programs
//! (the cores, which must reach their nodes), the local network and DHCP.

use std::path::PathBuf;

use anyhow::{bail, Result};
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::*;

/// TUN adapter addresses: Prism and sing-box use the first, mihomo its default.
const TUN_V4: [[u8; 4]; 2] = [[172, 19, 0, 1], [198, 18, 0, 1]];
const TUN_V6: [u8; 16] = [0xfd, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];

/// Private, link-local, multicast and broadcast ranges.
const LAN_V4: [([u8; 4], u32); 6] = [
    ([10, 0, 0, 0], 8),
    ([172, 16, 0, 0], 12),
    ([192, 168, 0, 0], 16),
    ([169, 254, 0, 0], 16),
    ([224, 0, 0, 0], 4),
    ([255, 255, 255, 255], 32),
];
const LAN_V6: [([u8; 2], u8); 3] = [([0xfe, 0x80], 10), ([0xfc, 0x00], 7), ([0xff, 0x00], 8)];

const SUBLAYER: GUID = GUID::from_u128(0x7f3a_91c2_5d44_4e1b_9a06_2b7c_e1d0_4a55);
const PERMIT_WEIGHT: u8 = 10;
const BLOCK_WEIGHT: u8 = 1;
const IPPROTO_UDP: u8 = 17;
const RPC_C_AUTHN_DEFAULT: u32 = 0xFFFF_FFFF;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn check(what: &str, code: u32) -> Result<()> {
    if code != 0 {
        bail!("{what} failed: 0x{code:08x}");
    }
    Ok(())
}

fn cond(field: GUID, match_type: FWP_MATCH_TYPE, value_type: FWP_DATA_TYPE, value: FWP_CONDITION_VALUE0_0) -> FWPM_FILTER_CONDITION0 {
    FWPM_FILTER_CONDITION0 { fieldKey: field, matchType: match_type, conditionValue: FWP_CONDITION_VALUE0 { r#type: value_type, Anonymous: value } }
}

/// A holder of the filters; dropping it removes them.
pub struct KillSwitch {
    engine: HANDLE,
    /// Programs with permit filters.
    allowed: Vec<PathBuf>,
}

// The handle isn't tied to a thread, and it is only used through `&mut`/drop.
unsafe impl Send for KillSwitch {}

impl KillSwitch {
    /// Adds the filters, all or none. `allowed` programs may connect anywhere.
    pub fn enable(allowed: &[PathBuf]) -> Result<Self> {
        let session = FWPM_SESSION0 { flags: FWPM_SESSION_FLAG_DYNAMIC, ..unsafe { std::mem::zeroed() } };
        let mut engine: HANDLE = std::ptr::null_mut();
        check("FwpmEngineOpen0", unsafe { FwpmEngineOpen0(std::ptr::null(), RPC_C_AUTHN_DEFAULT, std::ptr::null(), &session, &mut engine) })?;
        // From here on, dropping closes the session and undoes everything.
        let mut ks = KillSwitch { engine, allowed: Vec::new() };
        check("FwpmTransactionBegin0", unsafe { FwpmTransactionBegin0(engine, 0) })?;
        ks.add_filters(allowed)?;
        check("FwpmTransactionCommit0", unsafe { FwpmTransactionCommit0(engine) })?;
        ks.allowed = allowed.to_vec();
        Ok(ks)
    }

    /// Lets programs not allowed yet connect too (a core installed while
    /// the block is up), all or none.
    pub fn allow(&mut self, programs: &[PathBuf]) -> Result<()> {
        let new: Vec<PathBuf> = programs.iter().filter(|p| !self.allowed.contains(p)).cloned().collect();
        if new.is_empty() {
            return Ok(());
        }
        check("FwpmTransactionBegin0", unsafe { FwpmTransactionBegin0(self.engine, 0) })?;
        let r = [FWPM_LAYER_ALE_AUTH_CONNECT_V4, FWPM_LAYER_ALE_AUTH_CONNECT_V6]
            .into_iter()
            .try_for_each(|layer| new.iter().try_for_each(|path| self.permit_app(layer, path)));
        if let Err(e) = r.and_then(|()| check("FwpmTransactionCommit0", unsafe { FwpmTransactionCommit0(self.engine) })) {
            // The engine must not stay in a transaction, or every later one fails.
            unsafe { FwpmTransactionAbort0(self.engine) };
            return Err(e);
        }
        self.allowed.extend(new);
        Ok(())
    }

    fn add(&self, layer: GUID, action: FWP_ACTION_TYPE, weight: u8, conds: &mut [FWPM_FILTER_CONDITION0]) -> Result<()> {
        let mut name = wide("Prism kill switch");
        let mut filter: FWPM_FILTER0 = unsafe { std::mem::zeroed() };
        filter.displayData.name = name.as_mut_ptr();
        filter.layerKey = layer;
        filter.subLayerKey = SUBLAYER;
        filter.weight = FWP_VALUE0 { r#type: FWP_UINT8, Anonymous: FWP_VALUE0_0 { uint8: weight } };
        filter.numFilterConditions = conds.len() as u32;
        filter.filterCondition = if conds.is_empty() { std::ptr::null_mut() } else { conds.as_mut_ptr() };
        filter.action.r#type = action;
        check("FwpmFilterAdd0", unsafe { FwpmFilterAdd0(self.engine, &filter, std::ptr::null_mut(), std::ptr::null_mut()) })
    }

    fn permit(&self, layer: GUID, conds: &mut [FWPM_FILTER_CONDITION0]) -> Result<()> {
        self.add(layer, FWP_ACTION_PERMIT, PERMIT_WEIGHT, conds)
    }

    /// A program, wherever it connects.
    fn permit_app(&self, layer: GUID, path: &PathBuf) -> Result<()> {
        let p = wide(&path.to_string_lossy());
        let mut blob: *mut FWP_BYTE_BLOB = std::ptr::null_mut();
        check("FwpmGetAppIdFromFileName0", unsafe { FwpmGetAppIdFromFileName0(p.as_ptr(), &mut blob) })?;
        let r = self.permit(layer, &mut [cond(FWPM_CONDITION_ALE_APP_ID, FWP_MATCH_EQUAL, FWP_BYTE_BLOB_TYPE, FWP_CONDITION_VALUE0_0 { byteBlob: blob })]);
        let mut raw = blob.cast::<core::ffi::c_void>();
        unsafe { FwpmFreeMemory0(&mut raw) };
        r
    }

    fn add_filters(&self, allowed: &[PathBuf]) -> Result<()> {
        let mut name = wide("Prism kill switch");
        let mut sub: FWPM_SUBLAYER0 = unsafe { std::mem::zeroed() };
        sub.subLayerKey = SUBLAYER;
        sub.displayData.name = name.as_mut_ptr();
        sub.weight = 0xFFFF;
        check("FwpmSubLayerAdd0", unsafe { FwpmSubLayerAdd0(self.engine, &sub, std::ptr::null_mut()) })?;

        let (v4, v6) = (FWPM_LAYER_ALE_AUTH_CONNECT_V4, FWPM_LAYER_ALE_AUTH_CONNECT_V6);
        for (layer, dhcp_port) in [(v4, 67u16), (v6, 547u16)] {
            // Loopback.
            self.permit(
                layer,
                &mut [cond(FWPM_CONDITION_FLAGS, FWP_MATCH_FLAGS_ALL_SET, FWP_UINT32, FWP_CONDITION_VALUE0_0 { uint32: FWP_CONDITION_FLAG_IS_LOOPBACK })],
            )?;
            // The cores, wherever they connect.
            for path in allowed {
                self.permit_app(layer, path)?;
            }
            // DHCP keeps the physical link usable.
            self.permit(
                layer,
                &mut [
                    cond(FWPM_CONDITION_IP_PROTOCOL, FWP_MATCH_EQUAL, FWP_UINT8, FWP_CONDITION_VALUE0_0 { uint8: IPPROTO_UDP }),
                    cond(FWPM_CONDITION_IP_REMOTE_PORT, FWP_MATCH_EQUAL, FWP_UINT16, FWP_CONDITION_VALUE0_0 { uint16: dhcp_port }),
                ],
            )?;
            // Everything else leaving this layer stops here.
            self.add(layer, FWP_ACTION_BLOCK, BLOCK_WEIGHT, &mut [])?;
        }

        for addr in TUN_V4 {
            let v = u32::from_be_bytes(addr);
            self.permit(v4, &mut [cond(FWPM_CONDITION_IP_LOCAL_ADDRESS, FWP_MATCH_EQUAL, FWP_UINT32, FWP_CONDITION_VALUE0_0 { uint32: v })])?;
        }
        for (addr, bits) in LAN_V4 {
            let mut m = FWP_V4_ADDR_AND_MASK { addr: u32::from_be_bytes(addr), mask: if bits == 0 { 0 } else { u32::MAX << (32 - bits) } };
            self.permit(v4, &mut [cond(FWPM_CONDITION_IP_REMOTE_ADDRESS, FWP_MATCH_EQUAL, FWP_V4_ADDR_MASK, FWP_CONDITION_VALUE0_0 { v4AddrMask: &mut m })])?;
        }
        let mut tun6 = FWP_BYTE_ARRAY16 { byteArray16: TUN_V6 };
        self.permit(v6, &mut [cond(FWPM_CONDITION_IP_LOCAL_ADDRESS, FWP_MATCH_EQUAL, FWP_BYTE_ARRAY16_TYPE, FWP_CONDITION_VALUE0_0 { byteArray16: &mut tun6 })])?;
        for (head, bits) in LAN_V6 {
            let mut addr = [0u8; 16];
            addr[..2].copy_from_slice(&head);
            let mut m = FWP_V6_ADDR_AND_MASK { addr, prefixLength: bits };
            self.permit(v6, &mut [cond(FWPM_CONDITION_IP_REMOTE_ADDRESS, FWP_MATCH_EQUAL, FWP_V6_ADDR_MASK, FWP_CONDITION_VALUE0_0 { v6AddrMask: &mut m })])?;
        }
        Ok(())
    }
}

impl Drop for KillSwitch {
    fn drop(&mut self) {
        unsafe { FwpmEngineClose0(self.engine) };
    }
}
