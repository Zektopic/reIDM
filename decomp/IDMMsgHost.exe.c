// decompiled by Ghidra 12.1.4
// program: IDMMsgHost.exe  sha256: dba25a49adb88f675db26d2dc7a0fa9d4a5db2326858cb9d2515f6f34b8e0b65

// ---- FUN_00401000 @ 00401000 ----

void __cdecl
FUN_00401000(undefined4 param_1,undefined4 param_2,undefined4 param_3,undefined4 param_4)

{
  uint *in_EAX;
  int iVar1;
  uint *puVar2;
  uint local_2c [9];
  uint *local_8;
  
  puVar2 = local_2c;
  for (iVar1 = 9; iVar1 != 0; iVar1 = iVar1 + -1) {
    *puVar2 = *in_EAX;
    in_EAX = in_EAX + 1;
    puVar2 = puVar2 + 1;
  }
  local_2c[0] = local_2c[0] & 0xf9930520 | 0x19930520;
  local_8 = local_2c;
  __CxxFrameHandler(param_1,param_2,param_3,param_4);
  return;
}



// ---- FUN_0040104d @ 0040104d ----

undefined * FUN_0040104d(void)

{
  return _iob_exref;
}



// ---- FUN_00401060 @ 00401060 ----

void __fastcall FUN_00401060(int param_1,undefined4 param_2)

{
  if (param_1 == DAT_00406000) {
    return;
  }
  FUN_0040172f(param_1,param_2);
  return;
}



// ---- __alloca_probe @ 00401070 ----

/* WARNING: This is an inlined function */

void __alloca_probe(void)

{
  undefined1 *in_EAX;
  undefined4 *puVar1;
  undefined4 *puVar2;
  undefined4 unaff_retaddr;
  undefined1 auStack_4 [4];
  
  puVar2 = (undefined4 *)((int)&stack0x00000000 - (int)in_EAX & ~-(uint)(&stack0x00000000 < in_EAX))
  ;
  for (puVar1 = (undefined4 *)((uint)auStack_4 & 0xfffff000); puVar2 < puVar1;
      puVar1 = puVar1 + -0x400) {
  }
  *puVar2 = unaff_retaddr;
  return;
}



// ---- __local_unwind4 @ 0040109c ----

/* Library Function - Single Match
    __local_unwind4
   
   Libraries: Visual Studio 2017 Debug, Visual Studio 2017 Release, Visual Studio 2019 Debug, Visual
   Studio 2019 Release */

void __cdecl __local_unwind4(uint *param_1,int param_2,uint param_3)

{
  undefined4 *puVar1;
  uint uVar2;
  void *pvStack_28;
  undefined1 *puStack_24;
  uint local_20;
  uint uStack_1c;
  int iStack_18;
  uint *puStack_14;
  
  puStack_14 = param_1;
  iStack_18 = param_2;
  uStack_1c = param_3;
  puStack_24 = &LAB_0040112c;
  pvStack_28 = ExceptionList;
  local_20 = DAT_00406000 ^ (uint)&pvStack_28;
  ExceptionList = &pvStack_28;
  while( true ) {
    uVar2 = *(uint *)(param_2 + 0xc);
    if ((uVar2 == 0xfffffffe) || ((param_3 != 0xfffffffe && (uVar2 <= param_3)))) break;
    puVar1 = (undefined4 *)((*(uint *)(param_2 + 8) ^ *param_1) + 0x10 + uVar2 * 0xc);
    *(undefined4 *)(param_2 + 0xc) = *puVar1;
    if (puVar1[1] == 0) {
      __NLG_Notify(0x101);
      FUN_004018e0();
    }
  }
  ExceptionList = pvStack_28;
  return;
}



// ---- FUN_00401172 @ 00401172 ----

void FUN_00401172(int param_1)

{
  __local_unwind4(*(uint **)(param_1 + 0x28),*(int *)(param_1 + 0x18),*(uint *)(param_1 + 0x1c));
  return;
}



// ---- _EH4_CallFilterFunc @ 0040118e ----

/* Library Function - Single Match
    @_EH4_CallFilterFunc@8
   
   Library: Visual Studio 2008 Release
   __fastcall _EH4_CallFilterFunc,8 */

void __fastcall _EH4_CallFilterFunc(undefined *param_1)

{
  (*(code *)param_1)();
  return;
}



// ---- _EH4_TransferToHandler @ 004011a5 ----

/* Library Function - Single Match
    @_EH4_TransferToHandler@8
   
   Library: Visual Studio 2008 Release
   __fastcall _EH4_TransferToHandler,8 */

void __fastcall _EH4_TransferToHandler(undefined *UNRECOVERED_JUMPTABLE)

{
  __NLG_Notify(1);
                    /* WARNING: Could not recover jumptable at 0x004011bc. Too many branches */
                    /* WARNING: Treating indirect jump as call */
  (*(code *)UNRECOVERED_JUMPTABLE)();
  return;
}



// ---- _EH4_GlobalUnwind @ 004011be ----

/* Library Function - Single Match
    @_EH4_GlobalUnwind@4
   
   Library: Visual Studio 2008 Release
   __fastcall _EH4_GlobalUnwind,4 */

void __fastcall _EH4_GlobalUnwind(PVOID param_1)

{
  RtlUnwind(param_1,(PVOID)0x4011d3,(PEXCEPTION_RECORD)0x0,(PVOID)0x0);
  return;
}



// ---- _EH4_LocalUnwind @ 004011d8 ----

/* Library Function - Single Match
    @_EH4_LocalUnwind@16
   
   Library: Visual Studio 2008 Release
   __fastcall _EH4_LocalUnwind,16 */

void __fastcall _EH4_LocalUnwind(int param_1,uint param_2,undefined4 param_3,uint *param_4)

{
  __local_unwind4(param_4,param_1,param_2);
  return;
}



// ---- __except_handler4_common @ 004011f0 ----

/* Library Function - Single Match
    __except_handler4_common
   
   Library: Visual Studio 2008 Release */

undefined4 __cdecl
__except_handler4_common
          (uint *param_1,undefined *param_2,int *param_3,PVOID param_4,undefined4 param_5)

{
  int iVar1;
  uint uVar2;
  PVOID pvVar3;
  int *piVar4;
  int *local_1c;
  undefined4 local_18;
  uint *local_14;
  undefined4 local_10;
  PVOID local_c;
  char local_5;
  
  piVar4 = (int *)(*(uint *)((int)param_4 + 8) ^ *param_1);
  local_5 = '\0';
  local_10 = 1;
  if (*piVar4 != -2) {
    (*(code *)param_2)();
  }
  (*(code *)param_2)();
  pvVar3 = param_4;
  if ((*(byte *)(param_3 + 1) & 0x66) == 0) {
    *(int ***)((int)param_4 + -4) = &local_1c;
    pvVar3 = *(PVOID *)((int)param_4 + 0xc);
    local_1c = param_3;
    local_18 = param_5;
    if (pvVar3 == (PVOID)0xfffffffe) {
      return local_10;
    }
    do {
      local_14 = (uint *)(piVar4 + (int)pvVar3 * 3 + 4);
      local_c = (PVOID)*local_14;
      if ((undefined *)piVar4[(int)pvVar3 * 3 + 5] != (undefined *)0x0) {
        iVar1 = _EH4_CallFilterFunc((undefined *)piVar4[(int)pvVar3 * 3 + 5]);
        local_5 = '\x01';
        if (iVar1 < 0) {
          local_10 = 0;
          goto LAB_00401298;
        }
        if (0 < iVar1) {
          if (((*param_3 == -0x1f928c9d) && (DAT_004066cc != (code *)0x0)) &&
             (uVar2 = FUN_00401974(0x4066cc), uVar2 != 0)) {
            (*DAT_004066cc)(param_3,1);
          }
          _EH4_GlobalUnwind(param_4);
          if (*(PVOID *)((int)param_4 + 0xc) != pvVar3) {
            _EH4_LocalUnwind((int)param_4,(uint)pvVar3,(int)param_4 + 0x10,param_1);
          }
          *(PVOID *)((int)param_4 + 0xc) = local_c;
          if (*piVar4 != -2) {
            (*(code *)param_2)();
          }
          (*(code *)param_2)();
          _EH4_TransferToHandler((undefined *)local_14[2]);
          goto LAB_00401353;
        }
      }
      pvVar3 = local_c;
    } while (local_c != (PVOID)0xfffffffe);
    if (local_5 == '\0') {
      return local_10;
    }
  }
  else {
LAB_00401353:
    if (*(int *)((int)pvVar3 + 0xc) == -2) {
      return local_10;
    }
    _EH4_LocalUnwind((int)pvVar3,0xfffffffe,(int)param_4 + 0x10,param_1);
  }
LAB_00401298:
  if (*piVar4 != -2) {
    (*(code *)param_2)();
  }
  (*(code *)param_2)();
  return local_10;
}



// ---- FUN_00401372 @ 00401372 ----

/* WARNING: Function: __SEH_prolog4 replaced with injection: SEH_prolog4 */
/* WARNING: Function: __SEH_epilog4 replaced with injection: EH_epilog3 */

_onexit_t __cdecl FUN_00401372(_onexit_t param_1)

{
  _onexit_t p_Var1;
  undefined4 local_24;
  int local_20 [5];
  undefined4 uStack_c;
  undefined *local_8;
  
  local_8 = &DAT_00404dc0;
  uStack_c = 0x40137e;
  local_20[0] = DAT_004066c8;
  if (DAT_004066c8 == -1) {
    p_Var1 = _onexit(param_1);
  }
  else {
    _lock(8);
    local_8 = (undefined *)0x0;
    local_20[0] = DAT_004066c8;
    local_24 = DAT_004066c4;
    p_Var1 = (_onexit_t)__dllonexit(param_1,local_20,&local_24);
    DAT_004066c8 = local_20[0];
    DAT_004066c4 = local_24;
    local_8 = (undefined *)0xfffffffe;
    FUN_004013ee();
  }
  return p_Var1;
}



// ---- FUN_004013ee @ 004013ee ----

void FUN_004013ee(void)

{
  _unlock(8);
  return;
}



// ---- FUN_004013f7 @ 004013f7 ----

int __cdecl FUN_004013f7(_onexit_t param_1)

{
  _onexit_t p_Var1;
  
  p_Var1 = FUN_00401372(param_1);
  return (p_Var1 != (_onexit_t)0x0) - 1;
}



// ---- FUN_0040140e @ 0040140e ----

type_info * __thiscall FUN_0040140e(void *this,byte param_1)

{
  type_info::~type_info(this);
  if ((param_1 & 1) != 0) {
    FUN_00401c50(this);
  }
  return this;
}



// ---- __initterm_e @ 00401430 ----

/* Library Function - Single Match
    __initterm_e
   
   Library: Visual Studio 2008 Release */

void __cdecl __initterm_e(undefined4 *param_1,undefined4 *param_2)

{
  int iVar1;
  
  iVar1 = 0;
  while ((param_1 < param_2 && (iVar1 == 0))) {
    if ((code *)*param_1 != (code *)0x0) {
      iVar1 = (*(code *)*param_1)();
    }
    param_1 = param_1 + 1;
  }
  return;
}



// ---- ___tmainCRTStartup @ 00401487 ----

/* WARNING: Function: __SEH_prolog4 replaced with injection: SEH_prolog4 */
/* WARNING: Function: __SEH_epilog4 replaced with injection: EH_epilog3 */
/* Library Function - Single Match
    ___tmainCRTStartup
   
   Library: Visual Studio 2008 Release */

int ___tmainCRTStartup(void)

{
  ushort uVar1;
  LONG LVar2;
  int iVar3;
  uint uVar4;
  _STARTUPINFOW local_70;
  ushort *local_28;
  int local_24;
  uint local_20;
  undefined4 uStack_c;
  undefined4 local_8;
  
  uStack_c = 0x401493;
  local_20 = 0;
  local_8 = 0;
  GetStartupInfoW(&local_70);
  local_8 = 1;
  iVar3 = *(int *)((int)Self + 4);
  local_24 = 0;
  do {
    LVar2 = InterlockedCompareExchange((LONG *)&DAT_004066bc,iVar3,0);
    if (LVar2 == 0) {
LAB_004014ed:
      if (DAT_004066b8 == 1) {
        _amsg_exit(0x1f);
      }
      else if (DAT_004066b8 == 0) {
        DAT_004066b8 = 1;
        iVar3 = __initterm_e((undefined4 *)&DAT_00404178,(undefined4 *)&DAT_00404184);
        if (iVar3 != 0) {
          return 0xff;
        }
      }
      else {
        DAT_00406374 = 1;
      }
      if (DAT_004066b8 == 1) {
        initterm(&DAT_00404164,&DAT_00404174);
        DAT_004066b8 = 2;
      }
      if (local_24 == 0) {
        InterlockedExchange((LONG *)&DAT_004066bc,0);
      }
      if ((DAT_004066c0 != (code *)0x0) && (uVar4 = FUN_00401974(0x4066c0), uVar4 != 0)) {
        (*DAT_004066c0)(0,2,0);
      }
      if (*(int *)_wcmdln_exref == 0) {
        return 0xff;
      }
      local_28 = *(ushort **)_wcmdln_exref;
      while ((uVar1 = *local_28, 0x20 < uVar1 || ((uVar1 != 0 && (local_20 != 0))))) {
        if (uVar1 == 0x22) {
          local_20 = (uint)(local_20 == 0);
        }
        local_28 = local_28 + 1;
      }
      for (; (*local_28 != 0 && (*local_28 < 0x21)); local_28 = local_28 + 1) {
      }
      DAT_00406370 = FUN_00402830();
      if (DAT_00406364 == 0) {
                    /* WARNING: Subroutine does not return */
        exit(DAT_00406370);
      }
      if (DAT_00406374 == 0) {
        _cexit();
        return DAT_00406370;
      }
      return DAT_00406370;
    }
    if (LVar2 == iVar3) {
      local_24 = 1;
      goto LAB_004014ed;
    }
    Sleep(1000);
  } while( true );
}



// ---- entry @ 00401725 ----

void entry(void)

{
  FUN_00401b94();
  ___tmainCRTStartup();
  return;
}



// ---- FUN_0040172f @ 0040172f ----

/* WARNING: Globals starting with '_' overlap smaller symbols at the same address */

void __fastcall FUN_0040172f(undefined4 param_1,undefined4 param_2)

{
  undefined4 in_EAX;
  HANDLE hProcess;
  undefined4 unaff_EBX;
  undefined4 unaff_EBP;
  undefined4 unaff_ESI;
  undefined4 unaff_EDI;
  undefined2 in_ES;
  undefined2 in_CS;
  undefined2 in_SS;
  undefined2 in_DS;
  undefined2 in_FS;
  undefined2 in_GS;
  byte in_AF;
  byte in_TF;
  byte in_IF;
  byte in_NT;
  byte in_AC;
  byte in_VIF;
  byte in_VIP;
  byte in_ID;
  undefined4 unaff_retaddr;
  UINT uExitCode;
  undefined4 local_32c;
  undefined4 local_328;
  
  _DAT_00406488 =
       (uint)(in_NT & 1) * 0x4000 | (uint)SBORROW4((int)&stack0xfffffffc,0x328) * 0x800 |
       (uint)(in_IF & 1) * 0x200 | (uint)(in_TF & 1) * 0x100 | (uint)((int)&local_32c < 0) * 0x80 |
       (uint)(&stack0x00000000 == (undefined1 *)0x32c) * 0x40 | (uint)(in_AF & 1) * 0x10 |
       (uint)((POPCOUNT((uint)&local_32c & 0xff) & 1U) == 0) * 4 |
       (uint)(&stack0xfffffffc < (undefined1 *)0x328) | (uint)(in_ID & 1) * 0x200000 |
       (uint)(in_VIP & 1) * 0x100000 | (uint)(in_VIF & 1) * 0x80000 | (uint)(in_AC & 1) * 0x40000;
  _DAT_0040648c = &stack0x00000004;
  _DAT_004063c8 = 0x10001;
  _DAT_00406378 = 0xc0000409;
  _DAT_0040637c = 1;
  local_32c = DAT_00406000;
  local_328 = DAT_00406004;
  _DAT_00406384 = unaff_retaddr;
  _DAT_00406454 = in_GS;
  _DAT_00406458 = in_FS;
  _DAT_0040645c = in_ES;
  _DAT_00406460 = in_DS;
  _DAT_00406464 = unaff_EDI;
  _DAT_00406468 = unaff_ESI;
  _DAT_0040646c = unaff_EBX;
  _DAT_00406470 = param_2;
  _DAT_00406474 = param_1;
  _DAT_00406478 = in_EAX;
  _DAT_0040647c = unaff_EBP;
  DAT_00406480 = unaff_retaddr;
  _DAT_00406484 = in_CS;
  _DAT_00406490 = in_SS;
  SetUnhandledExceptionFilter((LPTOP_LEVEL_EXCEPTION_FILTER)0x0);
  UnhandledExceptionFilter((_EXCEPTION_POINTERS *)&PTR_DAT_00404190);
  uExitCode = 0xc0000409;
  hProcess = GetCurrentProcess();
  TerminateProcess(hProcess,uExitCode);
  return;
}



// ---- __NLG_Notify @ 004018c1 ----

/* Library Function - Single Match
    __NLG_Notify
   
   Libraries: Visual Studio 2017 Debug, Visual Studio 2017 Release, Visual Studio 2019 Debug, Visual
   Studio 2019 Release */

void __NLG_Notify(ulong param_1)

{
  undefined4 in_EAX;
  undefined4 unaff_EBP;
  
  DAT_00406018 = param_1;
  DAT_00406014 = in_EAX;
  DAT_0040601c = unaff_EBP;
  return;
}



// ---- FUN_004018e0 @ 004018e0 ----

void FUN_004018e0(void)

{
  code *in_EAX;
  
  (*in_EAX)();
  return;
}



// ---- __ValidateImageBase @ 004018f0 ----

/* Library Function - Single Match
    __ValidateImageBase
   
   Libraries: Visual Studio 2008 Release, Visual Studio 2010 Release */

BOOL __cdecl __ValidateImageBase(PBYTE pImageBase)

{
  if ((*(short *)pImageBase == 0x5a4d) &&
     (*(int *)(pImageBase + *(int *)(pImageBase + 0x3c)) == 0x4550)) {
    return (uint)((short)*(int *)((int)(pImageBase + *(int *)(pImageBase + 0x3c)) + 0x18) == 0x10b);
  }
  return 0;
}



// ---- __FindPESection @ 00401930 ----

/* Library Function - Single Match
    __FindPESection
   
   Library: Visual Studio 2008 Release */

PIMAGE_SECTION_HEADER __cdecl __FindPESection(PBYTE pImageBase,DWORD_PTR rva)

{
  int iVar1;
  PIMAGE_SECTION_HEADER p_Var2;
  uint uVar3;
  
  iVar1 = *(int *)(pImageBase + 0x3c);
  uVar3 = 0;
  p_Var2 = (PIMAGE_SECTION_HEADER)
           (pImageBase + *(ushort *)(pImageBase + iVar1 + 0x14) + 0x18 + iVar1);
  if (*(ushort *)(pImageBase + iVar1 + 6) != 0) {
    do {
      if ((p_Var2->VirtualAddress <= rva) &&
         (rva < (p_Var2->Misc).PhysicalAddress + p_Var2->VirtualAddress)) {
        return p_Var2;
      }
      uVar3 = uVar3 + 1;
      p_Var2 = p_Var2 + 1;
    } while (uVar3 < *(ushort *)(pImageBase + iVar1 + 6));
  }
  return (PIMAGE_SECTION_HEADER)0x0;
}



// ---- FUN_00401974 @ 00401974 ----

/* WARNING: Function: __SEH_prolog4 replaced with injection: SEH_prolog4 */
/* WARNING: Function: __SEH_epilog4 replaced with injection: EH_epilog3 */

uint __cdecl FUN_00401974(int param_1)

{
  BOOL BVar1;
  PIMAGE_SECTION_HEADER p_Var2;
  
  BVar1 = __ValidateImageBase((PBYTE)&IMAGE_DOS_HEADER_00400000);
  if ((BVar1 != 0) &&
     (p_Var2 = __FindPESection((PBYTE)&IMAGE_DOS_HEADER_00400000,param_1 - 0x400000),
     p_Var2 != (PIMAGE_SECTION_HEADER)0x0)) {
    return ~(p_Var2->Characteristics >> 0x1f) & 1;
  }
  return 0;
}



// ---- __SEH_prolog4 @ 00401a14 ----

/* WARNING: This is an inlined function */
/* WARNING: Unable to track spacebase fully for stack */
/* WARNING: Variable defined which should be unmapped: param_2 */
/* Library Function - Single Match
    __SEH_prolog4
   
   Library: Visual Studio */

void __cdecl __SEH_prolog4(undefined4 param_1,int param_2)

{
  int iVar1;
  undefined4 unaff_EBX;
  undefined4 unaff_ESI;
  undefined4 unaff_EDI;
  undefined4 unaff_retaddr;
  uint auStack_1c [5];
  undefined1 local_8 [8];
  
  iVar1 = -param_2;
  *(undefined4 *)((int)auStack_1c + iVar1 + 0x10) = unaff_EBX;
  *(undefined4 *)((int)auStack_1c + iVar1 + 0xc) = unaff_ESI;
  *(undefined4 *)((int)auStack_1c + iVar1 + 8) = unaff_EDI;
  *(uint *)((int)auStack_1c + iVar1 + 4) = DAT_00406000 ^ (uint)&param_2;
  *(undefined4 *)((int)auStack_1c + iVar1) = unaff_retaddr;
  ExceptionList = local_8;
  return;
}



// ---- __SEH_epilog4 @ 00401a59 ----

/* WARNING: This is an inlined function */
/* Library Function - Single Match
    __SEH_epilog4
   
   Library: Visual Studio */

void __SEH_epilog4(void)

{
  undefined4 *unaff_EBP;
  undefined4 unaff_retaddr;
  
  ExceptionList = (void *)unaff_EBP[-4];
  *unaff_EBP = unaff_retaddr;
  return;
}



// ---- __except_handler4 @ 00401a6d ----

/* Library Function - Single Match
    __except_handler4
   
   Library: Visual Studio 2008 Release */

void __cdecl __except_handler4(int *param_1,PVOID param_2,undefined4 param_3)

{
  __except_handler4_common(&DAT_00406000,FUN_00401060,param_1,param_2,param_3);
  return;
}



// ---- FUN_00401a92 @ 00401a92 ----

undefined4 FUN_00401a92(int *param_1)

{
  int *piVar1;
  int iVar2;
  
  piVar1 = (int *)*param_1;
  if (((*piVar1 == -0x1f928c9d) && (piVar1[4] == 3)) &&
     ((iVar2 = piVar1[5], iVar2 == 0x19930520 ||
      (((iVar2 == 0x19930521 || (iVar2 == 0x19930522)) || (iVar2 == 0x1994000)))))) {
                    /* WARNING: Subroutine does not return */
    terminate();
  }
  return 0;
}



// ---- FUN_00401ae3 @ 00401ae3 ----

void FUN_00401ae3(void)

{
  _controlfp(0x10000,0x30000);
  return;
}



// ---- FUN_00401af6 @ 00401af6 ----

undefined4 FUN_00401af6(void)

{
  return 0;
}



// ---- FUN_00401af9 @ 00401af9 ----

/* WARNING: Function: __SEH_prolog4 replaced with injection: SEH_prolog4 */
/* WARNING: Function: __SEH_epilog4 replaced with injection: EH_epilog3 */

void FUN_00401af9(void)

{
  return;
}



// ---- FUN_00401b5a @ 00401b5a ----

undefined4 __cdecl FUN_00401b5a(undefined4 param_1)

{
  HMODULE pHVar1;
  int iVar2;
  
  pHVar1 = GetModuleHandleA((LPCSTR)0x0);
  if (pHVar1 != (HMODULE)0x0) {
    iVar2 = FUN_00401af9();
    if (iVar2 != 0) {
      if (*(short *)(iVar2 + 0x5c) == 2) {
        return 2;
      }
      if (*(short *)(iVar2 + 0x5c) == 3) {
        return 1;
      }
    }
  }
  return param_1;
}



// ---- FUN_00401b94 @ 00401b94 ----

void FUN_00401b94(void)

{
  uint uVar1;
  DWORD DVar2;
  DWORD DVar3;
  DWORD DVar4;
  uint uVar5;
  LARGE_INTEGER local_14;
  _FILETIME local_c;
  
  local_c.dwLowDateTime = 0;
  local_c.dwHighDateTime = 0;
  if ((DAT_00406000 == 0xbb40e64e) || ((DAT_00406000 & 0xffff0000) == 0)) {
    GetSystemTimeAsFileTime(&local_c);
    uVar5 = local_c.dwHighDateTime ^ local_c.dwLowDateTime;
    DVar2 = GetCurrentProcessId();
    DVar3 = GetCurrentThreadId();
    DVar4 = GetTickCount();
    QueryPerformanceCounter(&local_14);
    uVar5 = uVar5 ^ DVar2 ^ DVar3 ^ DVar4 ^ local_14.s.HighPart ^ local_14.s.LowPart;
    if ((uVar5 == 0xbb40e64e) ||
       (uVar1 = DAT_00406000 & 0xffff0000, DAT_00406000 = uVar5, uVar1 == 0)) {
      DAT_00406000 = 0xbb40e64f;
    }
  }
  DAT_00406004 = ~DAT_00406000;
  return;
}



// ---- FUN_00401c50 @ 00401c50 ----

void __cdecl FUN_00401c50(void *param_1)

{
  if (param_1 != (void *)0x0) {
                    /* WARNING: Could not recover jumptable at 0x00401c5e. Too many branches */
                    /* WARNING: Treating indirect jump as call */
    free(param_1);
    return;
  }
  return;
}



// ---- FUN_00401c70 @ 00401c70 ----

int __thiscall FUN_00401c70(void *this,int param_1)

{
  void *pvVar1;
  int iVar2;
  uint uVar3;
  size_t _NewSize;
  
  iVar2 = *(int *)((int)this + 4);
  uVar3 = param_1 + iVar2;
  if (*(uint *)((int)this + 8) < uVar3) {
    _NewSize = (uVar3 >> 2) + uVar3 + 0x10;
    pvVar1 = realloc(*(void **)this,_NewSize);
    if (pvVar1 == (void *)0x0) {
      return 0;
    }
    iVar2 = *(int *)((int)this + 4);
    *(void **)this = pvVar1;
    *(size_t *)((int)this + 8) = _NewSize;
  }
  return *(int *)this + iVar2;
}



// ---- FUN_00401cc0 @ 00401cc0 ----

void __thiscall FUN_00401cc0(void *this,char param_1)

{
  if (*(void **)((int)this + 4) != (void *)0x0) {
    free(*(void **)((int)this + 4));
  }
  if (param_1 != '\0') {
    *(undefined4 *)((int)this + 4) = 0;
    *(undefined4 *)((int)this + 8) = 0;
  }
  return;
}



// ---- FUN_00401cf0 @ 00401cf0 ----

undefined4 * __thiscall FUN_00401cf0(void *this,uint param_1)

{
  *(undefined ***)this = &PTR_FUN_00404660;
  if (*(void **)((int)this + 4) != (void *)0x0) {
    free(*(void **)((int)this + 4));
  }
  if ((param_1 & 1) != 0) {
    if ((param_1 & 4) == 0) {
      free(this);
      return this;
    }
    FUN_00402240();
  }
  return this;
}



// ---- FUN_00401d40 @ 00401d40 ----

void __fastcall FUN_00401d40(undefined4 *param_1)

{
  void *_Memory;
  
  _Memory = (void *)param_1[1];
  *param_1 = &PTR_FUN_00404660;
  if (_Memory == (void *)param_1[3]) {
    param_1[1] = 0;
    return;
  }
  if (_Memory != (void *)0x0) {
    free(_Memory);
  }
  return;
}



// ---- FUN_00401d70 @ 00401d70 ----

void __thiscall FUN_00401d70(void *this,char param_1)

{
  void *_Memory;
  
  _Memory = *(void **)((int)this + 4);
  if (_Memory != *(void **)((int)this + 0xc)) {
    if (_Memory != (void *)0x0) {
      free(_Memory);
    }
    if (param_1 != '\0') {
      *(undefined4 *)((int)this + 4) = *(undefined4 *)((int)this + 0xc);
      *(undefined4 *)((int)this + 8) = 0;
    }
  }
  return;
}



// ---- FUN_00401db0 @ 00401db0 ----

undefined4 * __thiscall FUN_00401db0(void *this,uint param_1)

{
  void *_Memory;
  
  _Memory = *(void **)((int)this + 4);
  *(undefined ***)this = &PTR_FUN_00404660;
  if (_Memory == *(void **)((int)this + 0xc)) {
    *(undefined4 *)((int)this + 4) = 0;
  }
  else if (_Memory != (void *)0x0) {
    free(_Memory);
  }
  if ((param_1 & 1) != 0) {
    if ((param_1 & 4) == 0) {
      free(this);
      return this;
    }
    FUN_00402240();
  }
  return this;
}



// ---- FUN_00401e10 @ 00401e10 ----

void __fastcall FUN_00401e10(int *param_1)

{
  void *_Memory;
  
  if ((*param_1 != (int)param_1 + 0x12) &&
     (_Memory = (void *)(*param_1 + -4), _Memory != (void *)0x0)) {
    free(_Memory);
  }
  return;
}



// ---- FUN_00401e30 @ 00401e30 ----

int * __fastcall FUN_00401e30(int *param_1)

{
  *param_1 = (int)param_1 + 0x12;
  param_1[1] = 0;
  param_1[2] = 0;
  param_1[3] = 0;
  param_1[4] = 0;
  *(undefined1 *)(param_1 + 3) = 0xff;
  *(undefined2 *)*param_1 = 0;
  FUN_00401e70(param_1,((int)(char)param_1[3] >> 0x1f & 0x105U) + 0x105);
  return param_1;
}



// ---- FUN_00401e70 @ 00401e70 ----

uint __thiscall FUN_00401e70(void *this,uint param_1)

{
  void *pvVar1;
  undefined2 *puVar2;
  uint uVar3;
  void *_Memory;
  
  uVar3 = *(uint *)((int)this + 8);
  if (uVar3 < param_1) {
    _Memory = (void *)0x0;
    if (*(int *)this != (int)this + 0x12) {
      _Memory = (void *)(*(int *)this + -4);
    }
    pvVar1 = realloc(_Memory,param_1 + 0x14);
    if (pvVar1 != (void *)0x0) {
      puVar2 = (undefined2 *)((int)pvVar1 + 4);
      if (_Memory == (void *)0x0) {
        *puVar2 = 0;
      }
      if (puVar2 != (undefined2 *)0x0) {
        *(undefined2 **)this = puVar2;
        *(uint *)((int)this + 8) = param_1 + 0x10;
        return param_1 + 0x10;
      }
    }
    uVar3 = 0;
  }
  return uVar3;
}



// ---- FUN_00401ed0 @ 00401ed0 ----

LPWSTR __fastcall FUN_00401ed0(undefined4 *param_1)

{
  size_t _Size;
  uint uVar1;
  LPWSTR _Memory;
  LPWSTR pWVar2;
  undefined **local_20;
  LPWSTR local_1c;
  size_t local_18;
  LPWSTR local_14;
  void *local_10;
  undefined1 *puStack_c;
  undefined4 local_8;
  
  puStack_c = &LAB_004038bd;
  local_10 = ExceptionList;
  ExceptionList = &local_10;
  local_20 = &PTR_FUN_00404658;
  local_1c = (LPWSTR)0x0;
  local_18 = 0;
  local_14 = (LPWSTR)0x0;
  local_8 = 0;
  _Size = FUN_00402110(param_1[1] + 1,*(char *)(param_1 + 3));
  _Memory = (LPWSTR)0x0;
  if (_Size == 0) {
LAB_00401f6d:
    pWVar2 = (LPWSTR)0x0;
    if (_Memory != local_14) goto LAB_00401f79;
  }
  else {
    FUN_00401d70(&local_20,'\0');
    local_1c = malloc(_Size);
    local_18 = 0;
    _Memory = local_1c;
    if ((local_1c == (LPWSTR)0x0) ||
       (local_18 = _Size,
       uVar1 = FUN_00402130(*(char *)(param_1 + 3),(LPCSTR)*param_1,param_1[1] + 1,_Size,local_1c,
                            _Size), _Memory = local_1c, (int)uVar1 < 1)) goto LAB_00401f6d;
    local_14 = local_1c;
    pWVar2 = local_1c;
  }
  _Memory = (LPWSTR)0x0;
  local_1c = (LPWSTR)0x0;
LAB_00401f79:
  local_20 = &PTR_FUN_00404660;
  if (_Memory != (LPWSTR)0x0) {
    free(_Memory);
  }
  ExceptionList = local_10;
  return pWVar2;
}



// ---- FUN_00401fb0 @ 00401fb0 ----

uint __thiscall FUN_00401fb0(void *this,wchar_t *param_1,size_t param_2,uint param_3)

{
  uint uVar1;
  void *extraout_ECX;
  void *extraout_ECX_00;
  LPWSTR pWVar2;
  undefined **local_20;
  LPWSTR local_1c;
  undefined4 local_18;
  LPWSTR local_14;
  void *local_10;
  undefined1 *puStack_c;
  undefined4 local_8;
  
  puStack_c = &LAB_004038ed;
  local_10 = ExceptionList;
  uVar1 = DAT_00406000 ^ (uint)&stack0xfffffffc;
  if (param_3 <= *(uint *)((int)this + 4)) {
    pWVar2 = *(LPWSTR *)this;
    local_18 = 0;
    local_20 = &PTR_FUN_00404658;
    local_8 = 0;
    ExceptionList = &local_10;
    local_1c = pWVar2;
    local_14 = pWVar2;
    if (*(uint *)((int)this + 4) == 0) {
      FUN_00401d70(&local_20,'\0');
      pWVar2 = (LPWSTR)0x0;
      this = extraout_ECX;
    }
    else if (-1 < *(char *)((int)this + 0xc)) {
      pWVar2 = FUN_00401ed0(this);
      (*(code *)local_20[1])(0,uVar1);
      this = extraout_ECX_00;
    }
    local_18 = 0;
    local_1c = pWVar2;
    uVar1 = FUN_004021a0(pWVar2,param_3,param_1,this,'\x01',param_2);
    local_20 = &PTR_FUN_00404660;
    if (local_1c == local_14) {
      local_1c = (LPWSTR)0x0;
    }
    if (local_1c != (LPWSTR)0x0) {
      free(local_1c);
    }
    ExceptionList = local_10;
    return uVar1;
  }
  return 0xffffffff;
}



// ---- FUN_004020a0 @ 004020a0 ----

undefined4 __thiscall FUN_004020a0(void *this,uint param_1)

{
  uint uVar1;
  uint3 uVar2;
  bool bVar3;
  
  if (*(uint *)((int)this + 4) == 0) {
    if (*(int *)((int)this + 8) == 0) {
      uVar1 = 0;
    }
    else {
      uVar1 = (*(int *)((int)this + 8) - 1U) / (((int)*(char *)((int)this + 0xc) >> 0x1f & 1U) + 1);
    }
    bVar3 = param_1 <= uVar1;
  }
  else {
    bVar3 = param_1 < *(uint *)((int)this + 4);
  }
  uVar2 = (uint3)(param_1 >> 8);
  if (bVar3) {
    *(uint *)((int)this + 4) = param_1;
    if (*(char *)((int)this + 0xc) < '\0') {
      *(undefined2 *)(*(int *)this + param_1 * 2) = 0;
      return CONCAT31(uVar2,1);
    }
    *(undefined1 *)(*(int *)this + param_1) = 0;
    return CONCAT31(uVar2,1);
  }
  return (uint)uVar2 << 8;
}



// ---- FUN_00402110 @ 00402110 ----

int __fastcall FUN_00402110(int param_1,char param_2)

{
  int iVar1;
  
  iVar1 = (((int)param_2 >> 0x1f & 1U) + 1) * param_1;
  if (param_2 != -1) {
    iVar1 = iVar1 * 2;
  }
  return iVar1;
}



// ---- FUN_00402130 @ 00402130 ----

uint __fastcall
FUN_00402130(char param_1,LPCSTR param_2,uint param_3,undefined4 param_4,LPWSTR param_5,uint param_6
            )

{
  uint uVar1;
  
  uVar1 = param_6 >> 1;
  if ((((param_2 != (LPCSTR)0x0) && (param_3 != 0)) && (param_5 != (LPWSTR)0x0)) && (uVar1 != 0)) {
    if (param_1 != -1) {
      uVar1 = MultiByteToWideChar(*(UINT *)(&DAT_004043fc + param_1 * 4),0,param_2,param_3,param_5,
                                  uVar1);
      return uVar1;
    }
    if (param_3 < uVar1) {
      uVar1 = param_3;
    }
    memcpy(param_5,param_2,uVar1 * 2);
    return uVar1;
  }
  return 0;
}



// ---- FUN_004021a0 @ 004021a0 ----

uint __fastcall
FUN_004021a0(short *param_1,int param_2,wchar_t *param_3,undefined4 param_4,char param_5,
            size_t param_6)

{
  wchar_t wVar1;
  uint uVar2;
  wchar_t *_Str1;
  bool bVar3;
  
  if (param_1 == (short *)0x0) {
    if (((param_3 != (wchar_t *)0x0) && (param_6 != 0)) && (*param_3 != L'\0')) {
      return 0xffffffff;
    }
  }
  else if (param_3 == (wchar_t *)0x0) {
    if ((param_6 != 0) && (*param_1 != 0)) {
      return 1;
    }
  }
  else {
    _Str1 = param_1 + param_2;
    if (-1 < (int)param_6) {
      if (param_5 == '\0') {
        uVar2 = wcsncmp(_Str1,param_3,param_6);
        return uVar2;
      }
      uVar2 = _wcsnicmp(_Str1,param_3,param_6);
      return uVar2;
    }
    if (param_5 != '\0') {
      uVar2 = _wcsicmp(_Str1,param_3);
      return uVar2;
    }
    do {
      wVar1 = *_Str1;
      bVar3 = (ushort)wVar1 < (ushort)*param_3;
      if (wVar1 != *param_3) {
LAB_00402216:
        return -(uint)bVar3 | 1;
      }
      if (wVar1 == L'\0') {
        return 0;
      }
      wVar1 = _Str1[1];
      bVar3 = (ushort)wVar1 < (ushort)param_3[1];
      if (wVar1 != param_3[1]) goto LAB_00402216;
      _Str1 = _Str1 + 2;
      param_3 = param_3 + 2;
    } while (wVar1 != L'\0');
  }
  return 0;
}



// ---- FUN_00402240 @ 00402240 ----

void FUN_00402240(void)

{
  return;
}



// ---- FUN_00402250 @ 00402250 ----

void __fastcall FUN_00402250(undefined4 param_1,uint param_2)

{
  BOOL BVar1;
  undefined4 extraout_EDX;
  _OSVERSIONINFOW local_124;
  ushort local_10;
  ushort local_e;
  char local_a;
  uint local_8;
  
  local_8 = DAT_00406000 ^ (uint)&stack0xfffffffc;
  if (DAT_004066a4 == 0) {
    local_124.dwOSVersionInfoSize = 0x11c;
    BVar1 = GetVersionExW(&local_124);
    if (BVar1 == 0) {
      FUN_00401060(local_8 ^ (uint)&stack0xfffffffc,extraout_EDX);
      return;
    }
    param_2 = ((local_124.dwMajorVersion << 8 | local_124.dwMinorVersion) << 8 | (uint)local_10) <<
              8 | (uint)local_e;
    DAT_004066a4 = param_2;
    if ((((local_124.dwMajorVersion == 6) && (local_124.dwMinorVersion == 0)) && (local_10 == 0)) &&
       (local_a != '\x01')) {
      DAT_004066a4 = param_2 | 0x100;
    }
  }
  FUN_00401060(local_8 ^ (uint)&stack0xfffffffc,param_2);
  return;
}



// ---- FUN_00402300 @ 00402300 ----

DWORD FUN_00402300(void)

{
  HMODULE hModule;
  HANDLE pvVar1;
  int iVar2;
  uint uVar3;
  HANDLE hProcess;
  BOOL BVar4;
  undefined4 extraout_ECX;
  uint extraout_EDX;
  _FILETIME *lpCreationTime;
  LPFILETIME lpExitTime;
  LPFILETIME lpKernelTime;
  LPFILETIME lpUserTime;
  undefined4 uVar5;
  DWORD *pDVar6;
  undefined4 uVar7;
  _FILETIME _Stack_44;
  _FILETIME a_Stack_3c [3];
  DWORD local_20 [7];
  
  if (DAT_004066a8 == (FARPROC)0x0) {
    hModule = GetModuleHandleW(L"NTDLL.DLL");
    if (hModule != (HMODULE)0x0) {
      DAT_004066a8 = GetProcAddress(hModule,"NtQueryInformationProcess");
    }
    if (DAT_004066a8 == (FARPROC)0x0) {
      return 0;
    }
  }
  pDVar6 = local_20;
  uVar7 = 0x18;
  uVar5 = 0;
  pvVar1 = GetCurrentProcess();
  iVar2 = (*DAT_004066a8)(pvVar1,uVar5,pDVar6,uVar7);
  if (iVar2 < 0) {
    return 0;
  }
  uVar3 = FUN_00402250(extraout_ECX,extraout_EDX);
  pvVar1 = OpenProcess((-(uint)(uVar3 < 0x6000000) & 0xfffff400) + 0x1000,0,local_20[0]);
  if (pvVar1 != (HANDLE)0x0) {
    lpExitTime = (LPFILETIME)&stack0xffffffac;
    lpCreationTime = a_Stack_3c;
    lpKernelTime = lpExitTime;
    lpUserTime = lpExitTime;
    hProcess = GetCurrentProcess();
    BVar4 = GetProcessTimes(hProcess,lpCreationTime,lpExitTime,lpKernelTime,lpUserTime);
    if ((BVar4 == 0) ||
       (BVar4 = GetProcessTimes(pvVar1,&_Stack_44,(LPFILETIME)&stack0xffffffb4,
                                (LPFILETIME)&stack0xffffffac,(LPFILETIME)&stack0xffffffac),
       BVar4 == 0)) {
      CloseHandle(pvVar1);
    }
    else {
      CloseHandle(pvVar1);
      if ((int)a_Stack_3c[0].dwHighDateTime < (int)_Stack_44.dwHighDateTime) {
        return 0;
      }
      if (((int)a_Stack_3c[0].dwHighDateTime <= (int)_Stack_44.dwHighDateTime) &&
         (a_Stack_3c[0].dwLowDateTime <= _Stack_44.dwLowDateTime)) {
        return 0;
      }
    }
  }
  return local_20[0];
}



// ---- FUN_00402420 @ 00402420 ----

uint __fastcall FUN_00402420(byte *param_1,uint param_2,uint param_3)

{
  byte bVar1;
  byte bVar2;
  int iVar3;
  byte *pbVar4;
  byte *pbVar5;
  byte *pbVar6;
  byte *pbVar7;
  byte *pbVar8;
  uint local_c;
  
  local_c = 0;
  pbVar7 = param_1;
  if (param_1 < param_1 + param_2) {
    do {
      if ((param_3 != 0) && (param_3 <= local_c)) goto LAB_00402489;
      bVar1 = *pbVar7;
      pbVar7 = pbVar7 + 1;
      if ("uuuuuuuubtnufruuuuuuuuuuuuuuuuuu"[bVar1] == '\0') {
        iVar3 = 1;
      }
      else {
        iVar3 = (uint)("uuuuuuuubtnufruuuuuuuuuuuuuuuuuu"[bVar1] == 'u') * 4 + 2;
      }
      local_c = local_c + iVar3;
    } while (pbVar7 < param_1 + param_2);
  }
  if (param_3 != 0) {
LAB_00402489:
    if ((local_c != param_2) && (pbVar4 = param_1 + local_c, param_1 < pbVar7)) {
      pbVar8 = pbVar4 + -2;
      pbVar6 = pbVar4 + -6;
      do {
        if (pbVar4 <= pbVar7) {
          return local_c;
        }
        bVar1 = pbVar7[-1];
        pbVar7 = pbVar7 + -1;
        bVar2 = "uuuuuuuubtnufruuuuuuuuuuuuuuuuuu"[bVar1];
        if (bVar2 == 0) {
          pbVar5 = pbVar4 + -1;
          pbVar6 = pbVar6 + -1;
          pbVar8 = pbVar8 + -1;
          *pbVar5 = bVar1;
        }
        else if (bVar2 == 0x75) {
          if (pbVar6 < param_1) {
            return local_c;
          }
          pbVar6 = pbVar6 + -6;
          pbVar8 = pbVar8 + -6;
          pbVar4[-5] = 0x75;
          pbVar4[-0xffffffff00000004] = 0x30;
          pbVar4[-0xffffffff00000003] = 0x30;
          pbVar5 = pbVar4 + -6;
          *pbVar5 = 0x5c;
          bVar2 = "0123456789abcdef"[(int)(char)bVar1 >> 4];
          pbVar4[-1] = "0123456789abcdef"[(int)(char)bVar1 & 0xf];
          pbVar4[-2] = bVar2;
        }
        else {
          if (pbVar8 < param_1) {
            return local_c;
          }
          pbVar5 = pbVar4 + -2;
          pbVar6 = pbVar6 + -2;
          pbVar8 = pbVar8 + -2;
          pbVar4[-1] = bVar2;
          *pbVar5 = 0x5c;
        }
        pbVar4 = pbVar5;
      } while (param_1 < pbVar7);
    }
  }
  return local_c;
}



// ---- FUN_00402550 @ 00402550 ----

int __fastcall FUN_00402550(byte *param_1,int param_2,byte *param_3,byte *param_4)

{
  byte bVar1;
  byte *pbVar2;
  int iVar3;
  byte *pbVar4;
  byte *pbVar5;
  byte *pbVar6;
  bool bVar7;
  
  param_4 = param_4 + (int)param_3;
  bVar7 = param_1 == param_3;
  pbVar4 = param_1;
  pbVar5 = param_3;
  if (param_3 < param_4) {
    while (pbVar4 < param_1 + param_2) {
      bVar1 = *pbVar5;
      pbVar6 = pbVar5 + 1;
      if (bVar1 == 0) {
        if (!bVar7) {
          *pbVar4 = 0;
        }
        pbVar4 = pbVar4 + 1;
        break;
      }
      if (bVar1 == 0x5c) {
        param_3 = (byte *)(int)(char)*pbVar6;
        iVar3 = 1;
        pbVar2 = param_3;
        switch(param_3) {
        case (byte *)0x30:
        case (byte *)0x31:
        case (byte *)0x32:
        case (byte *)0x33:
          iVar3 = FUN_00402720(pbVar6,(int *)&param_3,3,'\b');
          pbVar2 = param_3;
          if (iVar3 == 0) {
            pbVar2 = (byte *)0x0;
          }
          break;
        case (byte *)0x61:
          pbVar2 = (byte *)0x7;
          break;
        case (byte *)0x62:
          pbVar2 = (byte *)0x8;
          break;
        case (byte *)0x66:
          pbVar2 = (byte *)0xc;
          break;
        case (byte *)0x6e:
          pbVar2 = (byte *)0xa;
          break;
        case (byte *)0x72:
          pbVar2 = (byte *)0xd;
          break;
        case (byte *)0x74:
          pbVar2 = (byte *)0x9;
          break;
        case (byte *)0x75:
          pbVar6 = pbVar5 + 2;
          iVar3 = FUN_00402720(pbVar6,(int *)&param_3,4,'\x10');
          if (iVar3 == 4) {
LAB_00402631:
            pbVar6 = pbVar5 + 2;
            pbVar2 = param_3;
          }
          else {
            pbVar2 = (byte *)0x0;
          }
          break;
        case (byte *)0x76:
          pbVar2 = (byte *)0xb;
          break;
        case (byte *)0x78:
          pbVar6 = pbVar5 + 2;
          iVar3 = FUN_00402720(pbVar6,(int *)&param_3,2,'\x10');
          if (iVar3 == 2) goto LAB_00402631;
          pbVar2 = (byte *)0x0;
        }
        pbVar6 = pbVar6 + iVar3;
        bVar7 = false;
        if (pbVar2 < (byte *)0x80) {
          if (pbVar2 != (byte *)0x0) {
            *pbVar4 = (byte)pbVar2;
            goto LAB_00402677;
          }
        }
        else {
          iVar3 = FUN_00402780(pbVar4,(ushort)pbVar2);
          pbVar4 = pbVar4 + iVar3;
        }
      }
      else {
        if (!bVar7) {
          *pbVar4 = bVar1;
        }
LAB_00402677:
        pbVar4 = pbVar4 + 1;
      }
      pbVar5 = pbVar6;
      if (param_4 <= pbVar6) {
        return (int)pbVar4 - (int)param_1;
      }
    }
  }
  return (int)pbVar4 - (int)param_1;
}



// ---- FUN_00402720 @ 00402720 ----

int __fastcall FUN_00402720(byte *param_1,int *param_2,int param_3,char param_4)

{
  byte bVar1;
  byte *pbVar2;
  int iVar3;
  byte *pbVar4;
  
  if (param_3 == 0) {
    pbVar4 = (byte *)0xffffffff;
  }
  else {
    pbVar4 = param_1 + param_3;
  }
  iVar3 = 0;
  pbVar2 = param_1;
  if (param_1 < pbVar4) {
    do {
      bVar1 = *pbVar2;
      if ((int)param_4 <= (int)(uint)(byte)(&DAT_00404538)[bVar1]) break;
      pbVar2 = pbVar2 + 1;
      iVar3 = iVar3 * param_4 + (uint)(byte)(&DAT_00404538)[bVar1];
    } while (pbVar2 < pbVar4);
  }
  *param_2 = iVar3;
  return (int)pbVar2 - (int)param_1;
}



// ---- FUN_00402780 @ 00402780 ----

undefined4 __fastcall FUN_00402780(byte *param_1,ushort param_2)

{
  byte bVar1;
  byte bVar2;
  
  if (param_2 < 0x80) {
    *param_1 = (byte)param_2;
    return 1;
  }
  bVar2 = (byte)param_2 & 0x3f | 0x80;
  bVar1 = (byte)(param_2 >> 6);
  if (0x7ff < param_2) {
    *param_1 = (byte)(param_2 >> 0xc) | 0xe0;
    param_1[2] = bVar2;
    param_1[1] = bVar1 & 0x3f | 0x80;
    return 3;
  }
  *param_1 = bVar1 | 0xc0;
  param_1[1] = bVar2;
  return 2;
}



// ---- FUN_004027f0 @ 004027f0 ----

void __fastcall FUN_004027f0(undefined4 *param_1)

{
  if ((void *)*param_1 != (void *)0x0) {
    free((void *)*param_1);
  }
  return;
}



// ---- FUN_00402800 @ 00402800 ----

undefined4 * __thiscall FUN_00402800(void *this,undefined4 param_1,DWORD param_2)

{
  *(undefined4 *)this = param_1;
  if (param_2 == 0xffffffff) {
    param_2 = GetLastError();
  }
  *(DWORD *)((int)this + 4) = param_2;
  return this;
}



// ---- FUN_00402830 @ 00402830 ----

void FUN_00402830(void)

{
  uint uVar1;
  int *piVar2;
  HKEY pHVar3;
  HMODULE pHVar4;
  HANDLE pvVar5;
  DWORD DVar6;
  LSTATUS LVar7;
  HRESULT HVar8;
  BOOL BVar9;
  HANDLE pvVar10;
  DWORD DVar11;
  int iVar12;
  LPCVOID pvVar13;
  int iVar14;
  uint uVar15;
  HKEY pHVar16;
  HKEY pHVar17;
  wchar_t *pwVar18;
  HANDLE *ppvVar19;
  WCHAR *pWVar20;
  uint uVar21;
  int iVar22;
  uint *this;
  HKEY *ppHVar23;
  _OVERLAPPED *p_Var24;
  DWORD *pSessionId;
  uint uStack_22c;
  undefined1 local_218 [8];
  undefined1 local_210 [8];
  undefined1 local_208 [8];
  undefined1 local_200 [8];
  undefined1 local_1f8 [8];
  undefined1 local_1f0 [8];
  undefined1 local_1e8 [8];
  undefined1 local_1e0 [8];
  undefined1 local_1d8 [8];
  undefined1 local_1d0 [8];
  undefined1 local_1c8 [8];
  undefined1 local_1c0 [8];
  int local_1b8 [6];
  HANDLE local_1a0;
  uint local_19c;
  _OVERLAPPED local_198;
  undefined4 local_184;
  HANDLE local_180;
  HANDLE local_17c;
  undefined4 local_178;
  char local_174 [4];
  HANDLE local_170;
  uint local_16c;
  _OVERLAPPED local_168;
  undefined4 local_154;
  undefined4 local_150;
  undefined4 local_14c;
  undefined4 local_148;
  char local_144 [4];
  int *local_140;
  HANDLE local_13c;
  undefined4 local_138;
  undefined4 local_134;
  undefined4 local_130;
  undefined4 local_12c;
  undefined4 local_128;
  HANDLE local_124;
  undefined4 local_120;
  undefined4 local_11c;
  undefined4 local_118;
  undefined4 local_114;
  undefined2 local_110;
  HANDLE local_10c;
  undefined4 local_108;
  undefined4 local_104;
  undefined4 local_100;
  undefined4 local_fc;
  undefined4 local_f8;
  HANDLE local_f4;
  undefined4 local_f0;
  HANDLE local_ec;
  HANDLE local_e8;
  undefined4 local_e4;
  char local_e0;
  int *local_dc;
  HANDLE local_d8;
  DWORD local_d4;
  uint local_d0;
  HANDLE local_c8;
  int *local_c4;
  HKEY local_c0;
  HKEY local_bc;
  uint local_b8 [4];
  HANDLE local_a8 [4];
  WCHAR local_98 [27];
  wchar_t local_62;
  undefined1 local_60 [72];
  uint local_18;
  undefined1 *local_14;
  void *local_10;
  undefined1 *puStack_c;
  undefined4 local_8;
  
  local_8 = 0xffffffff;
  puStack_c = &LAB_0040392b;
  local_10 = ExceptionList;
  uStack_22c = DAT_00406000 ^ (uint)&stack0xfffffffc;
  local_14 = (undefined1 *)&uStack_22c;
  ExceptionList = &local_10;
  local_18 = uStack_22c;
  pHVar4 = GetModuleHandleW(L"KERNEL32.DLL");
  if (pHVar4 != (HMODULE)0x0) {
    DAT_004066ac = GetProcAddress(pHVar4,"GetNamedPipeServerProcessId");
    DAT_004066b0 = GetProcAddress(pHVar4,"GetNamedPipeClientProcessId");
  }
  if (((DAT_004066ac == (FARPROC)0x0) || (DAT_004066b0 == (FARPROC)0x0)) &&
     (pHVar4 = LoadLibraryW(L"PSAPI.DLL"), pHVar4 != (HMODULE)0x0)) {
    DAT_004066b4 = GetProcAddress(pHVar4,"GetProcessImageFileNameW");
  }
  pwVar18 = L"\\\\.\\pipe\\IDMNetworkMonitor.";
  pWVar20 = local_98;
  for (iVar14 = 0xe; iVar14 != 0; iVar14 = iVar14 + -1) {
    *(undefined4 *)pWVar20 = *(undefined4 *)pwVar18;
    pwVar18 = pwVar18 + 2;
    pWVar20 = pWVar20 + 2;
  }
  memset(local_60,0,0x48);
  local_d8 = (HANDLE)0x0;
  local_1b8[0] = 0;
  local_1b8[1] = 0;
  local_1b8[2] = 0;
  FUN_00401c70(local_1b8,0x100000);
  local_8 = 0;
  local_1b8[3] = 0;
  local_1b8[4] = 0;
  local_1b8[5] = 0;
  FUN_00401c70(local_1b8 + 3,0x10000);
  local_1a0 = (HANDLE)0x0;
  local_198.Internal = 0;
  local_198.InternalHigh = 0;
  local_198.u.s.Offset = 0;
  local_198.u.s.OffsetHigh = 0;
  local_198.hEvent = (HANDLE)0x0;
  local_184 = 0;
  local_180 = (HANDLE)0x0;
  local_17c = (HANDLE)0x0;
  local_178 = 0;
  local_174[0] = '\0';
  local_174[1] = '\0';
  local_170 = (HANDLE)0x0;
  local_16c = 0;
  local_168.Internal = 0;
  local_168.InternalHigh = 0;
  local_168.u.s.Offset = 0;
  local_168.u.s.OffsetHigh = 0;
  local_168.hEvent = (HANDLE)0x0;
  local_154 = 0;
  local_150 = 0;
  local_14c = 0;
  local_148 = 0;
  local_144[0] = '\0';
  local_140 = local_1b8;
  local_134 = 0;
  local_130 = 0;
  local_12c = 0;
  local_128 = 0;
  local_124 = (HANDLE)0x0;
  local_120 = 0;
  local_11c = 0;
  local_118 = 0;
  local_114 = 0;
  local_110 = 0;
  local_10c = (HANDLE)0x0;
  local_108 = 0;
  local_104 = 0;
  local_100 = 0;
  local_fc = 0;
  local_f8 = 0;
  local_f4 = (HANDLE)0x0;
  local_f0 = 0;
  local_ec = (HANDLE)0x0;
  local_e8 = (HANDLE)0x0;
  local_e4 = 0;
  local_e0 = '\0';
  local_dc = local_1b8 + 3;
  local_19c = 0;
  local_13c = (HANDLE)0x0;
  local_138 = 0;
  local_8 = CONCAT31(local_8._1_3_,2);
  pvVar5 = OpenMutexW(0,0,L"Local\\IDMEventMonitor");
  if (pvVar5 == (HANDLE)0x0) {
    DVar6 = GetLastError();
    if (DVar6 == 2) {
      local_c0 = (HKEY)0x0;
      local_c8 = (HANDLE)0x4;
      LVar7 = RegOpenKeyExW((HKEY)0x80000002,L"Software\\Internet Download Manager",0,0x20019,
                            &local_bc);
      if (LVar7 == 0) {
        RegQueryValueExW(local_bc,L"InstallStatus",(LPDWORD)0x0,(LPDWORD)0x0,(LPBYTE)&local_c0,
                         (LPDWORD)&local_c8);
        RegCloseKey(local_bc);
        if (((local_c0 == (HKEY)0x0) || (local_c0 == (HKEY)0x3)) &&
           (HVar8 = FUN_00403750(), -1 < HVar8)) {
          FUN_00402800(local_1c0,"Launched manager via COM",0);
                    /* WARNING: Subroutine does not return */
          _CxxThrowException(local_1c0,(ThrowInfo *)&DAT_00404e44);
        }
      }
    }
  }
  else {
    CloseHandle(pvVar5);
  }
  pSessionId = &local_d4;
  local_d4 = 0;
  DVar6 = GetCurrentProcessId();
  BVar9 = ProcessIdToSessionId(DVar6,pSessionId);
  if (BVar9 == 0) {
    FUN_00402800(local_1c8,"Cannot determine session Id",0xffffffff);
                    /* WARNING: Subroutine does not return */
    _CxxThrowException(local_1c8,(ThrowInfo *)&DAT_00404e44);
  }
  _itow(local_d4,&local_62,10);
  pvVar5 = GetStdHandle(0xfffffff6);
  local_c8 = pvVar5;
  pvVar10 = GetStdHandle(0xfffffff5);
  if ((pvVar5 == (HANDLE)0xffffffff) || (pvVar10 == (HANDLE)0xffffffff)) {
    FUN_00402800(local_218,"Cannot open stdio",0xffffffff);
                    /* WARNING: Subroutine does not return */
    _CxxThrowException(local_218,(ThrowInfo *)&DAT_00404e44);
  }
  local_d8 = CreateFileW(local_98,0xc0000000,0,(LPSECURITY_ATTRIBUTES)0x0,3,0x60000000,(HANDLE)0x0);
  if (local_d8 == (HANDLE)0xffffffff) {
    FUN_00402800(local_1d0,"Cannot open pipe",0xffffffff);
                    /* WARNING: Subroutine does not return */
    _CxxThrowException(local_1d0,(ThrowInfo *)&DAT_00404e44);
  }
  uVar21 = 0;
  ppvVar19 = &local_168.hEvent;
  local_1a0 = pvVar5;
  local_170 = local_d8;
  local_13c = local_d8;
  local_10c = pvVar10;
  do {
    pvVar5 = CreateEventW((LPSECURITY_ATTRIBUTES)0x0,1,0,(LPCWSTR)0x0);
    pvVar10 = CreateEventW((LPSECURITY_ATTRIBUTES)0x0,1,0,(LPCWSTR)0x0);
    if ((pvVar5 == (HANDLE)0x0) || (pvVar10 == (HANDLE)0x0)) {
      FUN_00402800(local_210,"Cannot create events",0xffffffff);
                    /* WARNING: Subroutine does not return */
      _CxxThrowException(local_210,(ThrowInfo *)&DAT_00404e44);
    }
    ppvVar19[-0xc] = pvVar5;
    uVar21 = uVar21 + 1;
    *ppvVar19 = pvVar10;
    ppvVar19 = ppvVar19 + 0x19;
  } while (uVar21 < 2);
  pvVar5 = CreateSemaphoreW((LPSECURITY_ATTRIBUTES)0x0,0,1,(LPCWSTR)0x0);
  pvVar10 = CreateSemaphoreW((LPSECURITY_ATTRIBUTES)0x0,0,1,(LPCWSTR)0x0);
  if ((pvVar5 == (HANDLE)0x0) || (pvVar10 == (HANDLE)0x0)) {
    FUN_00402800(local_208,"Cannot create semaphores",0xffffffff);
                    /* WARNING: Subroutine does not return */
    _CxxThrowException(local_208,(ThrowInfo *)&DAT_00404e44);
  }
  local_17c = pvVar5;
  local_e8 = pvVar10;
  pvVar5 = CreateThread((LPSECURITY_ATTRIBUTES)0x0,0,FUN_004034a0,&local_198,0,(LPDWORD)0x0);
  if (pvVar5 == (HANDLE)0x0) {
    FUN_00402800(local_1d8,"Cannot create input thread",0xffffffff);
                    /* WARNING: Subroutine does not return */
    _CxxThrowException(local_1d8,(ThrowInfo *)&DAT_00404e44);
  }
  pvVar10 = CreateThread((LPSECURITY_ATTRIBUTES)0x0,0,FUN_004034a0,&local_104,0,(LPDWORD)0x0);
  if (pvVar10 == (HANDLE)0x0) {
    FUN_00402800(local_1e0,"Cannot create output thread",0xffffffff);
                    /* WARNING: Subroutine does not return */
    _CxxThrowException(local_1e0,(ThrowInfo *)&DAT_00404e44);
  }
  local_180 = pvVar5;
  local_ec = pvVar10;
  FUN_00403530(local_140,local_c8);
  local_19c = local_140[1];
LAB_00402cf0:
  if (local_174[0] != '\0') {
    local_b8[0] = 0;
    local_a8[0] = local_198.hEvent;
  }
  DVar6 = (DWORD)(local_174[0] != '\0');
  if (local_144[0] != '\0') {
    local_b8[DVar6] = 1;
    local_a8[DVar6] = local_168.hEvent;
    DVar6 = DVar6 + 1;
  }
  if ((char)local_110 != '\0') {
    local_b8[DVar6] = 2;
    local_a8[DVar6] = local_124;
    DVar6 = DVar6 + 1;
  }
  if (local_e0 != '\0') {
    local_b8[DVar6] = 3;
    local_a8[DVar6] = local_f4;
    DVar6 = DVar6 + 1;
  }
  iVar14 = 0;
  while (iVar14 < (int)DVar6) {
    DVar11 = WaitForMultipleObjects(DVar6,local_a8,0,(iVar14 != 0) - 1);
    if (DVar11 == 0xffffffff) {
      FUN_00402800(local_1e8,"Asynchronous I/O failed",0xffffffff);
                    /* WARNING: Subroutine does not return */
      _CxxThrowException(local_1e8,(ThrowInfo *)&DAT_00404e44);
    }
    if (DVar11 == 0x102) break;
    ResetEvent(local_a8[DVar11]);
    uVar21 = local_b8[DVar11] >> 1;
    if ((~(byte)local_b8[DVar11] & 1) == 0) {
      BVar9 = GetOverlappedResult((&local_170)[uVar21 * 0x19],&local_168 + uVar21 * 5,
                                  (LPDWORD)&local_c0,0);
      if (BVar9 == 0) goto LAB_00402e5a;
      (&local_16c)[uVar21 * 0x19] = (int)&local_c0->unused + (&local_16c)[uVar21 * 0x19];
      iVar14 = iVar14 + 1;
      local_144[uVar21 * 100] = '\0';
    }
    else {
      BVar9 = GetOverlappedResult((&local_1a0)[uVar21 * 0x19],&local_198 + uVar21 * 5,
                                  (LPDWORD)&local_c0,0);
      if (BVar9 == 0) {
LAB_00402e5a:
        DVar11 = GetLastError();
        if (DVar11 != 0x3e4) {
          FUN_00402800(local_1f0,"Overlapped I/O failed",0xffffffff);
                    /* WARNING: Subroutine does not return */
          _CxxThrowException(local_1f0,(ThrowInfo *)&DAT_00404e44);
        }
      }
      else {
        local_c4 = (&local_140)[uVar21 * 0x19];
        local_bc = local_c0;
        iVar12 = FUN_00401c70(local_c4,(int)local_c0);
        if (iVar12 != 0) {
          local_c4[1] = (int)&local_bc->unused + local_c4[1];
        }
        local_174[uVar21 * 100] = '\0';
      }
      iVar14 = iVar14 + 1;
    }
  }
  local_bc = (HKEY)0x0;
LAB_00402e6f:
  pHVar16 = local_bc;
LAB_00402e75:
  do {
    if ((HKEY)0x1 < pHVar16) goto LAB_00402cf0;
    iVar14 = (int)pHVar16 * 100;
    pHVar17 = pHVar16;
    if ((local_174[iVar14 + 1] == '\0') && (local_174[iVar14] == '\0')) {
      local_c4 = (int *)0x1;
LAB_00402eb0:
      this = (uint *)(&local_140)[(int)pHVar16 * 0x19];
      uVar21 = (&local_19c)[(int)pHVar16 * 0x19];
      uVar1 = this[1];
      if (uVar21 == 0) {
        if (uVar1 == 4) {
          local_c0 = *(HKEY *)*this;
          (&local_19c)[(int)pHVar16 * 0x19] = (uint)(*(HKEY *)*this + 1);
          if ((pHVar17 != (HKEY)0x0) && (iVar12 = FUN_00401c70(this,1), iVar12 != 0)) {
            this[1] = this[1] + 1;
            (&local_19c)[(int)pHVar16 * 0x19] = (&local_19c)[(int)pHVar16 * 0x19] + 1;
          }
          this = (uint *)(&local_140)[(int)pHVar16 * 0x19];
        }
        else {
          local_c0 = (HKEY)(4 - uVar1);
        }
LAB_00403073:
        DVar6 = 0;
        p_Var24 = &local_198 + (int)pHVar16 * 5;
        ppHVar23 = &local_c0;
        pHVar17 = local_c0;
        pvVar13 = (LPCVOID)FUN_00401c70(this,(int)local_c0);
        local_c4 = (int *)FUN_00403410((&local_1a0)[(int)pHVar16 * 0x19],pvVar13,(DWORD)pHVar17,
                                       (LPDWORD)ppHVar23,p_Var24,DVar6);
        pHVar3 = local_c0;
        if (local_c4 == (int *)0x0) goto LAB_00403142;
        piVar2 = (&local_140)[(int)pHVar16 * 0x19];
        iVar12 = FUN_00401c70(piVar2,(int)local_c0);
        pHVar17 = local_bc;
        if (iVar12 != 0) {
          piVar2 = piVar2 + 1;
          *piVar2 = (int)&pHVar3->unused + *piVar2;
        }
        goto LAB_00402eb0;
      }
      local_c0 = (HKEY)(uVar21 - uVar1);
      if (0 < (int)(uVar21 - uVar1)) goto LAB_00403073;
      if (local_c4 != (int *)0x0) {
        local_174[iVar14 + 1] = '\x01';
        local_d0 = *this;
        if (pHVar17 == (HKEY)0x0) {
          if (2 < uVar21 - 4) {
            pHVar17 = (HKEY)0x0;
            if ((*(char *)(local_d0 + 4) == '\"') && (((char *)(local_d0 + 4))[uVar21 - 5] == '\"'))
            {
              iVar12 = FUN_00402550((byte *)(local_d0 + 5),uVar21 - 6,(byte *)(local_d0 + 5),
                                    (byte *)(uVar21 - 6));
              *(int *)(local_d0 + 1) = iVar12;
              (&local_19c)[(int)pHVar16 * 0x19] = iVar12 + 5;
              (&local_16c)[(int)pHVar16 * 0x19] = 1;
              pHVar17 = local_bc;
            }
            goto LAB_00402f8f;
          }
          local_174[iVar14 + 1] = '\0';
          (&local_19c)[(int)pHVar16 * 0x19] = 0;
          this[1] = 0;
          pHVar16 = pHVar17;
          goto LAB_00402e75;
        }
        uVar21 = uVar21 - 5;
        local_d0 = FUN_00402420((byte *)(local_d0 + 5),uVar21,0);
        local_c4 = (&local_140)[(int)pHVar16 * 0x19];
        iVar22 = (local_d0 - uVar21) + 1;
        iVar12 = FUN_00401c70(local_c4,iVar22);
        if (iVar12 != 0) {
          local_c4[1] = local_c4[1] + iVar22;
        }
        iVar12 = *(&local_140)[(int)pHVar16 * 0x19];
        if (uVar21 < local_d0) {
          uVar21 = FUN_00402420((byte *)(iVar12 + 5),uVar21,(&local_140)[(int)pHVar16 * 0x19][2] - 5
                               );
        }
        *(undefined1 *)(uVar21 + 5 + iVar12) = 0x22;
        *(undefined1 *)(iVar12 + 4) = 0x22;
        *(uint *)*(&local_140)[(int)pHVar16 * 0x19] = uVar21 + 2;
        (&local_19c)[(int)pHVar16 * 0x19] = uVar21 + 6;
        pHVar17 = local_bc;
        goto LAB_00402f8f;
      }
LAB_00403142:
      DVar6 = GetLastError();
      if (DVar6 != 0x3e5) {
        FUN_00402800(local_1f8,"Read operation failed",0xffffffff);
                    /* WARNING: Subroutine does not return */
        _CxxThrowException(local_1f8,(ThrowInfo *)&DAT_00404e44);
      }
      local_174[iVar14] = '\x01';
      pHVar17 = local_bc;
    }
LAB_00402f8f:
    if (((local_144[iVar14] != '\0') || ((&local_19c)[(int)pHVar16 * 0x19] == 0)) ||
       (local_174[iVar14] != '\0')) break;
    while( true ) {
      piVar2 = (&local_140)[(int)pHVar16 * 0x19];
      uVar21 = (&local_19c)[(int)pHVar16 * 0x19];
      uVar1 = (&local_16c)[(int)pHVar16 * 0x19];
      uVar15 = uVar21;
      if ((uint)piVar2[1] <= uVar21) {
        uVar15 = piVar2[1];
      }
      local_c8 = (HANDLE)(uVar15 - uVar1);
      if ((int)local_c8 < 1) break;
      BVar9 = FUN_00403410((&local_170)[(int)pHVar16 * 0x19],(LPCVOID)(*piVar2 + uVar1),
                           (DWORD)local_c8,(LPDWORD)&local_c8,&local_168 + (int)pHVar16 * 5,1);
      if (BVar9 == 0) {
        DVar6 = GetLastError();
        if (DVar6 != 0x3e5) {
          FUN_00402800(local_200,"Write operation failed",0xffffffff);
                    /* WARNING: Subroutine does not return */
          _CxxThrowException(local_200,(ThrowInfo *)&DAT_00404e44);
        }
        local_144[iVar14] = '\x01';
        pHVar17 = local_bc;
        goto LAB_0040317f;
      }
      (&local_16c)[(int)pHVar16 * 0x19] = (&local_16c)[(int)pHVar16 * 0x19] + (int)local_c8;
    }
    pHVar17 = local_bc;
    if (uVar1 < uVar21) break;
    local_174[iVar14 + 1] = '\0';
    (&local_19c)[(int)pHVar16 * 0x19] = 0;
    (&local_16c)[(int)pHVar16 * 0x19] = 0;
    piVar2[1] = 0;
    pHVar16 = local_bc;
  } while( true );
LAB_0040317f:
  local_bc = (HKEY)((int)&pHVar17->unused + 1);
  goto LAB_00402e6f;
}



// ---- Catch@004031b9 @ 004031b9 ----

undefined1 * Catch_004031b9(void)

{
  return &LAB_004031bf;
}



// ---- FUN_00403410 @ 00403410 ----

BOOL __fastcall
FUN_00403410(HANDLE param_1,LPCVOID param_2,DWORD param_3,LPDWORD param_4,LPOVERLAPPED param_5,
            DWORD param_6)

{
  HANDLE hEvent;
  BOOL BVar1;
  
  if ((param_5 != (LPOVERLAPPED)0x0) && (param_5[1].InternalHigh != 0)) {
    if (param_4 != (LPDWORD)0x0) {
      *param_4 = 0;
    }
    hEvent = param_5->hEvent;
    param_5[1].u.s.OffsetHigh = param_6;
    param_5[1].Internal = (ULONG_PTR)param_1;
    (param_5->u).Pointer = param_2;
    param_5->Internal = 0x103;
    param_5->InternalHigh = param_3;
    BVar1 = ResetEvent(hEvent);
    if (BVar1 != 0) {
      BVar1 = ReleaseSemaphore(param_5[1].u.Pointer,1,(LPLONG)0x0);
      if (BVar1 != 0) {
        SetLastError(0x3e5);
      }
    }
    return 0;
  }
  if (param_6 != 0) {
    BVar1 = WriteFile(param_1,param_2,param_3,param_4,param_5);
    return BVar1;
  }
  BVar1 = ReadFile(param_1,param_2,param_3,param_4,param_5);
  return BVar1;
}



// ---- FUN_004034a0 @ 004034a0 ----

undefined4 FUN_004034a0(undefined4 *param_1)

{
  undefined4 *puVar1;
  DWORD DVar2;
  int iVar3;
  undefined4 uVar4;
  code *pcVar5;
  code *pcVar6;
  
  puVar1 = param_1;
  pcVar6 = WaitForSingleObject_exref;
  DVar2 = WaitForSingleObject((HANDLE)param_1[7],0xffffffff);
  pcVar5 = WriteFile_exref;
  while (DVar2 != 0x80) {
    if (puVar1[8] == 0) {
      iVar3 = ReadFile((HANDLE)puVar1[5],(LPVOID)puVar1[2],puVar1[1],(LPDWORD)&param_1,
                       (LPOVERLAPPED)0x0);
      pcVar5 = WriteFile_exref;
    }
    else {
      iVar3 = (*pcVar5)((HANDLE)puVar1[5],(LPVOID)puVar1[2],puVar1[1],&param_1);
      pcVar6 = WaitForSingleObject_exref;
    }
    uVar4 = 0xc0000008;
    if (iVar3 != 0) {
      uVar4 = 0;
    }
    *puVar1 = uVar4;
    puVar1[1] = param_1;
    SetEvent((HANDLE)puVar1[4]);
    DVar2 = (*pcVar6)(puVar1[7],0xffffffff);
  }
  return 0;
}



// ---- FUN_00403530 @ 00403530 ----

void __fastcall FUN_00403530(int *param_1,undefined4 param_2)

{
  char cVar1;
  uint uVar2;
  DWORD dwProcessId;
  HANDLE hObject;
  uint uVar3;
  int iVar4;
  int iVar5;
  char *pcVar6;
  char *pcVar7;
  undefined4 *puVar8;
  undefined1 local_44 [20];
  DWORD local_30;
  undefined1 *local_2c;
  int local_28;
  int local_24;
  char local_20;
  undefined1 local_1f;
  undefined1 local_1a [2];
  DWORD local_18;
  DWORD local_14;
  void *local_10;
  undefined1 *puStack_c;
  undefined4 local_8;
  
  local_8 = 0xffffffff;
  puStack_c = &LAB_0040396d;
  local_10 = ExceptionList;
  uVar2 = DAT_00406000 ^ (uint)&stack0xfffffffc;
  ExceptionList = &local_10;
  dwProcessId = FUN_00402300();
  if ((DAT_004066ac == (code *)0x0) || (DAT_004066b0 == (code *)0x0)) {
    if ((DAT_004066b4 != (code *)0x0) && (DAT_004066a8 != (code *)0x0)) {
      FUN_00401e30((int *)&local_2c);
      local_1f = 1;
      local_8 = 0;
      hObject = OpenProcess(0x400,0,dwProcessId);
      if (hObject != (HANDLE)0x0) {
        if (local_24 == 0) {
          uVar3 = 0;
        }
        else {
          uVar3 = (local_24 - 1U) / ((local_20 < '\0') + 1);
        }
        uVar2 = (*DAT_004066b4)(hObject,local_2c,uVar3,uVar2);
        FUN_004020a0(&local_2c,uVar2);
        if (-1 < (int)(local_28 - 8U)) {
          uVar2 = FUN_00401fb0(&local_2c,L"\\cmd.exe",8,local_28 - 8U);
          if (uVar2 == 0) {
            iVar4 = (*DAT_004066a8)(hObject,0,local_44,0x18,0);
            if (iVar4 == 0) {
              dwProcessId = local_30;
            }
          }
        }
        CloseHandle(hObject);
      }
      if ((local_2c != local_1a) && (local_2c + -4 != (undefined1 *)0x0)) {
        free(local_2c + -4);
      }
    }
  }
  else {
    local_14 = 0;
    local_18 = 0;
    (*DAT_004066ac)(param_2,&local_18);
    (*DAT_004066b0)(param_2,&local_14);
    if ((dwProcessId == local_14) && (local_18 != 0)) {
      dwProcessId = local_18;
    }
  }
  iVar4 = FUN_00401c70(param_1,4);
  if (iVar4 != 0) {
    param_1[1] = param_1[1] + 4;
  }
  iVar4 = param_1[1];
  if ((uint)param_1[2] < iVar4 + 0xfU) {
    iVar5 = FUN_00401c70(param_1,0xf);
    if (iVar5 != 0) goto LAB_0040369f;
  }
  else {
LAB_0040369f:
    puVar8 = (undefined4 *)(param_1[1] + *param_1);
    *puVar8 = DAT_00404924;
    puVar8[1] = DAT_00404928;
    puVar8[2] = DAT_0040492c;
    *(undefined2 *)(puVar8 + 3) = DAT_00404930;
    *(undefined1 *)((int)puVar8 + 0xe) = DAT_00404932;
    param_1[1] = iVar4 + 0xfU;
  }
  iVar4 = 10;
  pcVar6 = (char *)FUN_00401c70(param_1,0xb);
  pcVar7 = _itoa(dwProcessId,pcVar6,iVar4);
  pcVar6 = pcVar7 + 1;
  do {
    cVar1 = *pcVar7;
    pcVar7 = pcVar7 + 1;
  } while (cVar1 != '\0');
  iVar4 = FUN_00401c70(param_1,(int)pcVar7 - (int)pcVar6);
  if (iVar4 != 0) {
    param_1[1] = param_1[1] + ((int)pcVar7 - (int)pcVar6);
  }
  uVar2 = param_1[1] + 1;
  if ((uint)param_1[2] < uVar2) {
    iVar4 = FUN_00401c70(param_1,1);
    if (iVar4 == 0) {
      uVar2 = param_1[1];
      goto LAB_0040372f;
    }
  }
  *(undefined1 *)(param_1[1] + *param_1) = 0x3b;
  param_1[1] = uVar2;
LAB_0040372f:
  *(uint *)*param_1 = uVar2 - 4;
  ExceptionList = local_10;
  return;
}



// ---- FUN_00403750 @ 00403750 ----

HRESULT FUN_00403750(void)

{
  HRESULT HVar1;
  undefined4 local_28;
  undefined4 local_24;
  undefined4 local_20;
  undefined4 local_1c;
  undefined4 local_18;
  undefined4 local_14;
  undefined4 local_10;
  undefined4 local_c;
  int *local_8;
  
  HVar1 = CoInitialize((LPVOID)0x0);
  if (-1 < HVar1) {
    local_8 = (int *)0x0;
    HVar1 = CoCreateInstance((IID *)&DAT_00404648,(LPUNKNOWN)0x0,4,(IID *)&DAT_00404638,&local_8);
    if ((HVar1 == 0) && (local_8 != (int *)0x0)) {
      Ordinal_8(&local_28);
      Ordinal_8(&local_18);
      HVar1 = (**(code **)(*local_8 + 0x10))
                        (local_8,L"idmcmd://startnetmon/",0,0,0,0,0,0,0,8,local_28,local_24,local_20
                         ,local_1c,local_18,local_14,local_10,local_c);
      if (HVar1 == -0x7fffbfff) {
        HVar1 = 1;
      }
    }
    CoUninitialize();
  }
  return HVar1;
}



// ---- __acrt_iob_func @ 00403820 ----

undefined * __cdecl __acrt_iob_func(int param_1)

{
  int iVar1;
  
                    /* 0x3820  1  __acrt_iob_func */
  iVar1 = (*(code *)PTR_FUN_00404188)();
  return (undefined *)(iVar1 + param_1 * 0x20);
}



// ---- fstat32 @ 00403840 ----

/* fstat32 */

void __cdecl fstat32(void)

{
                    /* 0x3840  2  _fstat32 */
                    /* WARNING: Could not recover jumptable at 0x00403844. Too many branches */
                    /* WARNING: Treating indirect jump as call */
  _fstat();
  return;
}



// ---- stat32 @ 00403850 ----

/* stat32 */

void __cdecl stat32(void)

{
                    /* 0x3850  3  _stat32 */
                    /* WARNING: Could not recover jumptable at 0x00403854. Too many branches */
                    /* WARNING: Treating indirect jump as call */
  _stat();
  return;
}



// ---- _wstat32 @ 00403860 ----

int __cdecl _wstat32(wchar_t *_Name,_stat32 *_Stat)

{
  int iVar1;
  
                    /* 0x3860  4  _wstat32 */
                    /* WARNING: Could not recover jumptable at 0x00403864. Too many branches */
                    /* WARNING: Treating indirect jump as call */
  iVar1 = _wstat();
  return iVar1;
}



// ---- Unwind@004038b0 @ 004038b0 ----

void Unwind_004038b0(void)

{
  int unaff_EBP;
  
  FUN_00401d40((undefined4 *)(unaff_EBP + -0x1c));
  return;
}



// ---- Unwind@004038e0 @ 004038e0 ----

void Unwind_004038e0(void)

{
  int unaff_EBP;
  
  FUN_00401d40((undefined4 *)(unaff_EBP + -0x1c));
  return;
}



// ---- Unwind@00403910 @ 00403910 ----

void Unwind_00403910(void)

{
  int unaff_EBP;
  
  FUN_004027f0((undefined4 *)(unaff_EBP + -0x1b4));
  return;
}



// ---- Unwind@0040391b @ 0040391b ----

void Unwind_0040391b(void)

{
  int unaff_EBP;
  
  FUN_004027f0((undefined4 *)(unaff_EBP + -0x1a8));
  return;
}



// ---- Unwind@00403960 @ 00403960 ----

void Unwind_00403960(void)

{
  int unaff_EBP;
  
  FUN_00401e10((int *)(unaff_EBP + -0x28));
  return;
}


