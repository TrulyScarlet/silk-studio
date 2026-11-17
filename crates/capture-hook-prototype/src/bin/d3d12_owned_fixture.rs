fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(all(windows, target_arch = "x86_64", feature = "d3d12-cross-process"))]
    {
        let args: Vec<String> = std::env::args().collect();
        #[cfg(feature = "d3d12-remote-helper-fixture")]
        if args.iter().any(|arg| arg == "--observe-owned-loss") {
            capture_hook_prototype::d3d12_bootstrap::validate_owned_attached_process()?;
        }
        #[cfg(not(feature = "d3d12-remote-helper-fixture"))]
        if args.iter().any(|arg| arg == "--observe-owned-loss") {
            return Err("observed helper loss feature disabled".into());
        }
        #[cfg(feature = "resident-hooks")]
        if args.iter().any(|arg| arg == "--diagnose-owned-faults")
            && args.iter().any(|arg| arg == "--allow-owned-d3d12")
        {
            unsafe {
                windows::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter(Some(
                    report_owned_fault,
                ));
            }
        }
        if args.iter().any(|arg| arg == "--pending-media-removal") {
            if !args.iter().any(|arg| arg == "--allow-owned-d3d12")
                || args.iter().skip(1).any(|arg| {
                    !matches!(
                        arg.as_str(),
                        "--allow-owned-d3d12"
                            | "--pending-media-removal"
                            | "--warp"
                            | "--expose-local-consumer"
                            | "--repeat-pending-media-removal"
                    )
                })
            {
                return Err(
                    "pending-media removal requires standalone explicit owned-fixture opt-in"
                        .into(),
                );
            }
            #[cfg(feature = "d3d12-device-replace-fixture")]
            {
                let cycles = if args
                    .iter()
                    .any(|arg| arg == "--repeat-pending-media-removal")
                {
                    10
                } else {
                    1
                };
                for _ in 0..cycles {
                    capture_hook_prototype::d3d12_present_fixture::D3d12PresentFixture::probe_pending_media_removal(args.iter().any(|arg| arg == "--warp"), args.iter().any(|arg| arg == "--expose-local-consumer"))?;
                }
                return Ok(());
            }
            #[cfg(not(feature = "d3d12-device-replace-fixture"))]
            return Err("pending-media removal feature disabled".into());
        }
        if args.iter().any(|arg| arg == "--auto-resize-owned")
            && (!cfg!(feature = "d3d12-auto-resize-fixture")
                || !args.iter().any(|arg| arg == "--late-d3d12")
                || !args.iter().any(|arg| arg == "--passive-d3d12"))
        {
            return Err(
                "automatic resize requires the enabled exact passive late fixture route".into(),
            );
        }
        if args.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "--late-d3d12" | "--late-no-resize" | "--late-distinct-queues"
            )
        }) && !args.iter().any(|arg| arg == "--passive-d3d12")
        {
            return Err("late queue observation requires the exact passive fixture route".into());
        }
        if args.iter().any(|arg| arg == "--passive-d3d12") {
            #[cfg(feature = "d3d12-remote-helper-fixture")]
            return capture_hook_prototype::d3d12_present_fixture::D3d12PresentFixture::run_passive_renderer(args.iter().any(|arg|arg=="--warp")).map_err(Into::into);
            #[cfg(not(feature = "d3d12-remote-helper-fixture"))]
            return Err("D3D12 remote helper fixture feature not enabled".into());
        }
        if args
            .iter()
            .any(|arg| arg.starts_with("--fixture-lifetime-seconds="))
            && !args.iter().any(|arg| arg == "--bootstrap")
        {
            return Err(
                "extended fixture lifetime requires the exact owned bootstrap route".into(),
            );
        }
        if args.iter().any(|arg| arg == "--replace-device")
            && !args.iter().any(|arg| arg == "--bootstrap")
        {
            return Err("device replacement requires the exact owned bootstrap route".into());
        }
        #[cfg(feature = "d3d12-bootstrap-fixture")]
        if args.iter().any(|arg| arg == "--bootstrap") {
            return capture_hook_prototype::d3d12_bootstrap::run_owned_producer(
                args.iter().any(|arg| arg == "--warp"),
                args.iter().find_map(|arg| arg.strip_prefix("--fault=")),
            )
            .map_err(Into::into);
        }
        #[cfg(not(feature = "d3d12-bootstrap-fixture"))]
        if args.iter().any(|arg| arg == "--bootstrap") {
            return Err("bootstrap feature not enabled".into());
        }
        #[cfg(feature = "d3d12-factory-hook-fixture")]
        if args.iter().any(|arg| arg == "--factory-hook-probe") {
            return capture_hook_prototype::d3d12_factory_hook::run_owned_probe(
                args.iter().any(|arg| arg == "--warp"),
            )
            .map_err(Into::into);
        }
        if !args.iter().any(|a| a == "--producer")
            || !args.iter().any(|a| a == "--allow-owned-d3d12")
        {
            return Err("explicit owned D3D12 producer opt-in required".into());
        }
        let fault = args.iter().find_map(|a| a.strip_prefix("--fault="));
        capture_hook_prototype::d3d12_cross_process::run_producer(
            args.iter().any(|a| a == "--warp"),
            fault,
            args.iter().any(|a| a == "--present-hook"),
            args.iter().any(|a| a == "--factory-hook"),
        )
        .map_err(Into::into)
    }
    #[cfg(not(all(windows, target_arch = "x86_64", feature = "d3d12-cross-process")))]
    {
        Err("owned D3D12 fixture requires Windows x64".into())
    }
}

#[cfg(all(windows, target_arch = "x86_64", feature = "resident-hooks"))]
unsafe extern "system" fn report_owned_fault(
    info: *const windows::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS,
) -> i32 {
    use windows::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };
    let Some(info) = info.as_ref() else {
        return 0;
    };
    let Some(record) = info.ExceptionRecord.as_ref() else {
        return 0;
    };
    eprintln!(
        "OWNED FIXTURE unhandled fault code={:#x} address={:p}",
        record.ExceptionCode.0, record.ExceptionAddress
    );
    if let Some(context) = info.ContextRecord.as_ref() {
        use windows::Win32::System::Diagnostics::Debug::{
            ReadProcessMemory, SymFromAddr, SymInitialize, SYMBOL_INFO,
        };
        let process = windows::Win32::System::Threading::GetCurrentProcess();
        let _ = SymInitialize(process, windows::core::PCSTR::null(), true);
        let mut words = [0usize; 128];
        let mut bytes = 0;
        if ReadProcessMemory(
            process,
            context.Rsp as *const _,
            words.as_mut_ptr().cast(),
            std::mem::size_of_val(&words),
            Some(&mut bytes),
        )
        .is_ok()
        {
            for address in std::iter::once(context.Rip as usize)
                .chain(words.into_iter().take(bytes / std::mem::size_of::<usize>()))
            {
                let mut storage = [0u64; 160];
                let symbol = storage.as_mut_ptr().cast::<SYMBOL_INFO>();
                (*symbol).SizeOfStruct = std::mem::size_of::<SYMBOL_INFO>() as u32;
                (*symbol).MaxNameLen = 1024;
                let mut offset = 0;
                if SymFromAddr(process, address as u64, Some(&mut offset), symbol).is_ok() {
                    let name = std::slice::from_raw_parts(
                        (*symbol).Name.as_ptr().cast::<u8>(),
                        (*symbol).NameLen as usize,
                    );
                    eprintln!(
                        "  stack candidate {}+{offset:#x}",
                        String::from_utf8_lossy(name)
                    );
                }
            }
        }
    }
    let mut stack = [std::ptr::null_mut(); 32];
    let count =
        windows::Win32::System::Diagnostics::Debug::RtlCaptureStackBackTrace(0, &mut stack, None);
    for address in
        std::iter::once(record.ExceptionAddress).chain(stack.into_iter().take(count as usize))
    {
        let mut module = windows::Win32::Foundation::HMODULE::default();
        if GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            windows::core::PCWSTR(address.cast()),
            &mut module,
        )
        .is_ok()
        {
            let mut name = [0u16; 1024];
            let len = GetModuleFileNameW(Some(module), &mut name) as usize;
            eprintln!(
                "  {}+{:#x}",
                String::from_utf16_lossy(&name[..len]),
                (address as usize).wrapping_sub(module.0 as usize)
            );
        }
    }
    0 // Continue normal fatal exception handling; never hide the crash.
}
