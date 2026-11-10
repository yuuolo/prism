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
    tail_filters: Vec<u64>,
    tail_scope: bool,
    tail_blocks: Vec<u64>,
    tail_permit: Option<crate::tailguard::Permit>,
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
        let ks = KillSwitch { engine, tail_filters: vec![], tail_scope: false, tail_blocks: vec![], tail_permit: None };
        check("FwpmTransactionBegin0", unsafe { FwpmTransactionBegin0(engine, 0) })?;
        ks.add_filters(allowed)?;
        check("FwpmTransactionCommit0", unsafe { FwpmTransactionCommit0(engine) })?;
        Ok(ks)
    }

    fn add_id(&self, layer: GUID, action: FWP_ACTION_TYPE, weight: u8, conds: &mut [FWPM_FILTER_CONDITION0]) -> Result<u64> {
        let mut name = wide("Prism kill switch");
        let mut filter: FWPM_FILTER0 = unsafe { std::mem::zeroed() };
        filter.displayData.name = name.as_mut_ptr();
        filter.layerKey = layer;
        filter.subLayerKey = SUBLAYER;
        filter.weight = FWP_VALUE0 { r#type: FWP_UINT8, Anonymous: FWP_VALUE0_0 { uint8: weight } };
        filter.numFilterConditions = conds.len() as u32;
        filter.filterCondition = if conds.is_empty() { std::ptr::null_mut() } else { conds.as_mut_ptr() };
        filter.action.r#type = action;
        let mut id = 0;
        check("FwpmFilterAdd0", unsafe { FwpmFilterAdd0(self.engine, &filter, std::ptr::null_mut(), &mut id) })?;
        Ok(id)
    }

    fn add(&self, layer: GUID, action: FWP_ACTION_TYPE, weight: u8, conds: &mut [FWPM_FILTER_CONDITION0]) -> Result<()> {
        self.add_id(layer, action, weight, conds).map(|_| ())
    }

    /// Base internet blocks are never removed. Revoke old exceptions first;
    /// add a complete new exception set in a separate WFP transaction.
    pub fn set_tailnet(&mut self, requested: bool, permit: Option<crate::tailguard::Permit>) -> Result<()> {
        if self.tail_scope == requested && self.tail_permit.as_ref().map(|p| p.key()) == permit.as_ref().map(|p| p.key()) { return Ok(()); }
        while let Some(id) = self.tail_filters.last().copied() {
            check("撤销 Tailscale 例外", unsafe { FwpmFilterDeleteById0(self.engine,id) })?;
            self.tail_filters.pop();
        }
        self.tail_permit = None;
        if !requested {
            while let Some(id) = self.tail_blocks.last().copied() {
                check("撤销 Tailscale 地址限制",unsafe {FwpmFilterDeleteById0(self.engine,id)})?; self.tail_blocks.pop();
            }
            self.tail_scope = false; return Ok(());
        }
        if !self.tail_scope {
            let mut blocks = Vec::new();
            check("Tailscale scope transaction",unsafe {FwpmTransactionBegin0(self.engine,0)})?;
            if let Err(e)=self.tail_rules(None,&mut blocks) {unsafe {FwpmTransactionAbort0(self.engine);} return Err(e);}
            if let Err(e)=check("Tailscale scope commit",unsafe {FwpmTransactionCommit0(self.engine)}) {unsafe {FwpmTransactionAbort0(self.engine);} return Err(e);}
            self.tail_blocks=blocks; self.tail_scope=true;
        }
        if permit.is_none() { return Ok(()); }
        let mut ids = Vec::new();
        check("Tailscale transaction", unsafe { FwpmTransactionBegin0(self.engine,0) })?;
        let result = self.tail_rules(permit.as_ref(), &mut ids);
        if let Err(e) = result { unsafe { FwpmTransactionAbort0(self.engine); } return Err(e); }
        if let Err(e) = check("Tailscale commit", unsafe { FwpmTransactionCommit0(self.engine) }) { unsafe { FwpmTransactionAbort0(self.engine); } return Err(e); }
        self.tail_filters = ids; self.tail_scope = true; self.tail_permit = permit;
        Ok(())
    }
    fn tail_rules(&self, permit: Option<&crate::tailguard::Permit>, ids: &mut Vec<u64>) -> Result<()> {
        let (v4,v6) = (FWPM_LAYER_ALE_AUTH_CONNECT_V4,FWPM_LAYER_ALE_AUTH_CONNECT_V6);
        // More specific than the existing LAN/ULA permits. When Tailscale is
        // offline these ranges cannot fall through to a physical interface.
        if !self.tail_scope {
        let mut range4 = FWP_V4_ADDR_AND_MASK {addr: u32::from_be_bytes([100,64,0,0]),mask:u32::MAX<<22};
        ids.push(self.add_id(v4,FWP_ACTION_BLOCK,20,&mut [cond(FWPM_CONDITION_IP_REMOTE_ADDRESS,FWP_MATCH_EQUAL,FWP_V4_ADDR_MASK,FWP_CONDITION_VALUE0_0{v4AddrMask:&mut range4})])?);
        let mut range6 = FWP_V6_ADDR_AND_MASK {addr:[0xfd,0x7a,0x11,0x5c,0xa1,0xe0,0,0,0,0,0,0,0,0,0,0],prefixLength:48};
        ids.push(self.add_id(v6,FWP_ACTION_BLOCK,20,&mut [cond(FWPM_CONDITION_IP_REMOTE_ADDRESS,FWP_MATCH_EQUAL,FWP_V6_ADDR_MASK,FWP_CONDITION_VALUE0_0{v6AddrMask:&mut range6})])?);
        }
        let Some(permit) = permit else { return Ok(()); };
        let path = wide(&permit.identity.daemon.to_string_lossy());
        let mut blob: *mut FWP_BYTE_BLOB = std::ptr::null_mut();
        check("Tailscale AppId",unsafe {FwpmGetAppIdFromFileName0(path.as_ptr(),&mut blob)})?;
        let mut descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let sddl=wide("D:(A;;CC;;;SY)");
        let ok=unsafe {windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(),1,&mut descriptor,std::ptr::null_mut())};
        if ok==0 { let mut raw=blob.cast(); unsafe {FwpmFreeMemory0(&mut raw)}; bail!("Tailscale SYSTEM identity descriptor failed"); }
        let mut sd_blob=FWP_BYTE_BLOB{size:unsafe {windows_sys::Win32::Security::GetSecurityDescriptorLength(descriptor)},data:descriptor.cast()};
        let result: Result<()> = (|| {
            for layer in [v4,v6] {
                // Only the verified daemon: peer UDP transport, HTTPS control/DERP.
                for protocol in [6u8,17u8] {
                    let mut conditions=vec![cond(FWPM_CONDITION_ALE_USER_ID,FWP_MATCH_EQUAL,FWP_SECURITY_DESCRIPTOR_TYPE,FWP_CONDITION_VALUE0_0{sd:&mut sd_blob}),cond(FWPM_CONDITION_ALE_APP_ID,FWP_MATCH_EQUAL,FWP_BYTE_BLOB_TYPE,FWP_CONDITION_VALUE0_0{byteBlob:blob}),cond(FWPM_CONDITION_IP_PROTOCOL,FWP_MATCH_EQUAL,FWP_UINT8,FWP_CONDITION_VALUE0_0{uint8:protocol})];
                    if protocol==6 {conditions.push(cond(FWPM_CONDITION_IP_REMOTE_PORT,FWP_MATCH_EQUAL,FWP_UINT16,FWP_CONDITION_VALUE0_0{uint16:443}));}
                    ids.push(self.add_id(layer,FWP_ACTION_PERMIT,30,&mut conditions)?);
                }
            }
            Ok(())
        })();
        let mut raw=blob.cast(); unsafe {FwpmFreeMemory0(&mut raw); windows_sys::Win32::Foundation::LocalFree(descriptor); } result?;
        let Some(interface)=&permit.interface else {return Ok(())};
        for ip in &permit.peers {
            let mut luid=interface.luid;
            let mut conditions=vec![cond(FWPM_CONDITION_IP_LOCAL_INTERFACE,FWP_MATCH_EQUAL,FWP_UINT64,FWP_CONDITION_VALUE0_0{uint64:&mut luid})];
            match ip {
                std::net::IpAddr::V4(ip)=>{
                    conditions.push(cond(FWPM_CONDITION_IP_REMOTE_ADDRESS,FWP_MATCH_EQUAL,FWP_UINT32,FWP_CONDITION_VALUE0_0{uint32:u32::from_be_bytes(ip.octets())}));
                    ids.push(self.add_id(v4,FWP_ACTION_PERMIT,30,&mut conditions)?);
                }
                std::net::IpAddr::V6(ip)=>{
                    let mut addr=FWP_BYTE_ARRAY16{byteArray16:ip.octets()};
                    conditions.push(cond(FWPM_CONDITION_IP_REMOTE_ADDRESS,FWP_MATCH_EQUAL,FWP_BYTE_ARRAY16_TYPE,FWP_CONDITION_VALUE0_0{byteArray16:&mut addr}));
                    ids.push(self.add_id(v6,FWP_ACTION_PERMIT,30,&mut conditions)?);
                }
            }
        }
        Ok(())
    }

    fn permit(&self, layer: GUID, conds: &mut [FWPM_FILTER_CONDITION0]) -> Result<()> {
        self.add(layer, FWP_ACTION_PERMIT, PERMIT_WEIGHT, conds)
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
                let p = wide(&path.to_string_lossy());
                let mut blob: *mut FWP_BYTE_BLOB = std::ptr::null_mut();
                check("FwpmGetAppIdFromFileName0", unsafe { FwpmGetAppIdFromFileName0(p.as_ptr(), &mut blob) })?;
                let r = self.permit(layer, &mut [cond(FWPM_CONDITION_ALE_APP_ID, FWP_MATCH_EQUAL, FWP_BYTE_BLOB_TYPE, FWP_CONDITION_VALUE0_0 { byteBlob: blob })]);
                let mut raw = blob.cast::<core::ffi::c_void>();
                unsafe { FwpmFreeMemory0(&mut raw) };
                r?;
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


#[cfg(test)]
mod tailnet_tests {
    use super::*;
    fn guid(a:GUID,b:GUID)->bool {a.data1==b.data1 && a.data2==b.data2 && a.data3==b.data3 && a.data4==b.data4}
    #[test]
    fn native_scope_filters_and_revocation_keep_base_protection() {
        // This briefly enables real global WFP protection. Only run on an
        // explicitly opted-in disposable CI machine, never normal user tests.
        if std::env::var("PRISM_TAILGUARD_WFP_TEST").as_deref()!=Ok("1") {return;}
        let exe=std::env::current_exe().unwrap();
        let mut ks=KillSwitch::enable(&[exe.clone()]).unwrap();
        let engine=ks.engine;
        ks.set_tailnet(true,None).unwrap();
        let blocks=ks.tail_blocks.clone(); assert_eq!(blocks.len(),2);
        let permit=crate::tailguard::Permit {
            identity:crate::tailguard::fixture_identity(&exe),
            interface:Some(crate::tailguard::Interface{index:777,luid:777,addresses:vec!["100.64.1.1".parse().unwrap()]}),
            peers:vec!["100.64.1.2".parse().unwrap(),"fd7a:115c:a1e0::2".parse().unwrap()],
        };
        ks.set_tailnet(true,Some(permit)).unwrap();
        let allows=ks.tail_filters.clone(); assert_eq!(allows.len(),6);
        let mut daemon=0; let mut peers=0;
        for id in &allows {unsafe {
            let mut raw=std::ptr::null_mut(); assert_eq!(FwpmFilterGetById0(ks.engine,*id,&mut raw),0);
            let filter=&*raw;
            assert_eq!(filter.action.r#type,FWP_ACTION_PERMIT);
            let c=std::slice::from_raw_parts(filter.filterCondition,filter.numFilterConditions as usize);
            if c.iter().any(|x|guid(x.fieldKey,FWPM_CONDITION_ALE_APP_ID)) {
                daemon+=1;
                assert!(c.iter().any(|x|guid(x.fieldKey,FWPM_CONDITION_ALE_USER_ID)));
                let protocol=c.iter().find(|x|guid(x.fieldKey,FWPM_CONDITION_IP_PROTOCOL)).unwrap().conditionValue.Anonymous.uint8;
                assert!(protocol==6 || protocol==17);
                if protocol==6 {
                    assert_eq!(c.len(),4);
                    assert_eq!(c.iter().find(|x|guid(x.fieldKey,FWPM_CONDITION_IP_REMOTE_PORT)).unwrap().conditionValue.Anonymous.uint16,443);
                } else {assert_eq!(c.len(),3);}
            } else {
                peers+=1;
                assert!(c.iter().any(|x|guid(x.fieldKey,FWPM_CONDITION_IP_LOCAL_INTERFACE)));
                assert!(c.iter().any(|x|guid(x.fieldKey,FWPM_CONDITION_IP_REMOTE_ADDRESS)));
                assert_eq!(c.len(),2);
                assert_eq!(*c.iter().find(|x|guid(x.fieldKey,FWPM_CONDITION_IP_LOCAL_INTERFACE)).unwrap().conditionValue.Anonymous.uint64,777);
            }
            let mut memory=raw.cast(); FwpmFreeMemory0(&mut memory);
        }}
        assert_eq!((daemon,peers),(4,2));
        ks.set_tailnet(true,None).unwrap();
        assert_eq!(ks.engine,engine); assert_eq!(ks.tail_blocks,blocks); assert!(ks.tail_filters.is_empty());
        for id in allows {unsafe {let mut raw=std::ptr::null_mut();assert_ne!(FwpmFilterGetById0(ks.engine,id,&mut raw),0);}}
        for id in blocks {unsafe {let mut raw=std::ptr::null_mut();assert_eq!(FwpmFilterGetById0(ks.engine,id,&mut raw),0);let mut memory=raw.cast();FwpmFreeMemory0(&mut memory);}}
        ks.set_tailnet(false,None).unwrap(); assert!(ks.tail_blocks.is_empty()); assert_eq!(ks.engine,engine);
    }
}
