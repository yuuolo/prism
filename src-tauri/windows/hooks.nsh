; Prism registers prism:// itself (HKCU) and, when asked to, takes over
; clash:// links, keeping the previous command in the key's PrismPrevious
; value (src-tauri/src/links.rs). On uninstall - not on updates - both go
; back to how they were. The Run entry for "start with Windows" is named
; after the product, so the stock uninstaller already removes it.

!macro PRISM_RELEASE_SCHEME SCHEME
  ReadRegStr $R7 HKCU "Software\Classes\${SCHEME}\shell\open\command" ""
  ${If} $R7 == "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
    ReadRegStr $R8 HKCU "Software\Classes\${SCHEME}" "PrismPrevious"
    ${If} $R8 == ""
      DeleteRegKey HKCU "Software\Classes\${SCHEME}"
    ${Else}
      WriteRegStr HKCU "Software\Classes\${SCHEME}\shell\open\command" "" $R8
      DeleteRegValue HKCU "Software\Classes\${SCHEME}" "PrismPrevious"
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    !insertmacro PRISM_RELEASE_SCHEME "prism"
    !insertmacro PRISM_RELEASE_SCHEME "clash"
  ${EndIf}
!macroend
