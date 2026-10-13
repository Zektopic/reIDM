// decompiled by Ghidra 12.1.4
// program: idmnmcl.dll  sha256: 6baf9aa997be0e4fc2b31f896999fdaeba64946b0bdd336a659732b3bc0450b5

// ---- FUN_10001000 @ 10001000 ----

int FUN_10001000(void)

{
  int iVar1;
  uint in_EAX;
  void *pvVar2;
  undefined4 *unaff_ESI;
  
  iVar1 = in_EAX + 0x10 + (in_EAX >> 2);
  pvVar2 = realloc((void *)*unaff_ESI,iVar1 + 1);
  if (pvVar2 == (void *)0x0) {
    return 0;
  }
  unaff_ESI[2] = iVar1;
  *unaff_ESI = pvVar2;
  return unaff_ESI[1] + (int)pvVar2;
}



// ---- FUN_10001030 @ 10001030 ----

void FUN_10001030(void)

{
  undefined1 *_Src;
  int iVar1;
  LONG LVar2;
  undefined4 *puVar3;
  int *unaff_EBX;
  int iVar4;
  
  _Src = (undefined1 *)*unaff_EBX;
  if (1 < *(int *)(_Src + -0xc)) {
    if (_Src != &DAT_100085b8) {
      LVar2 = InterlockedDecrement((LONG *)(_Src + -0xc));
      if (LVar2 < 1) {
        operator_delete((void *)(*unaff_EBX + -0xc));
      }
      *unaff_EBX = (int)PTR_DAT_100085c0;
    }
    iVar1 = *(int *)(_Src + -8);
    if (iVar1 == 0) {
      *unaff_EBX = (int)PTR_DAT_100085c0;
    }
    else {
      iVar4 = ((int)(iVar1 + (iVar1 >> 0x1f & 3U)) >> 2) + iVar1;
      puVar3 = operator_new(iVar4 + 0xd);
      *puVar3 = 1;
      *(undefined1 *)((int)puVar3 + iVar1 + 0xc) = 0;
      puVar3[2] = iVar4;
      puVar3[1] = iVar1;
      *unaff_EBX = (int)(puVar3 + 3);
    }
    memcpy((void *)*unaff_EBX,_Src,*(int *)(_Src + -8) + 1);
  }
  return;
}



// ---- FUN_100010d0 @ 100010d0 ----

void FUN_100010d0(void)

{
  undefined1 *puVar1;
  LONG LVar2;
  undefined4 *puVar3;
  int *unaff_EBX;
  int iVar4;
  int unaff_EDI;
  
  puVar1 = (undefined1 *)*unaff_EBX;
  if ((1 < *(int *)(puVar1 + -0xc)) || (*(int *)(puVar1 + -4) < unaff_EDI)) {
    if (puVar1 != &DAT_100085b8) {
      LVar2 = InterlockedDecrement((LONG *)(puVar1 + -0xc));
      if (LVar2 < 1) {
        operator_delete((void *)(*unaff_EBX + -0xc));
      }
      *unaff_EBX = (int)PTR_DAT_100085c0;
    }
    if (unaff_EDI == 0) {
      *unaff_EBX = (int)PTR_DAT_100085c0;
      return;
    }
    iVar4 = ((int)(unaff_EDI + (unaff_EDI >> 0x1f & 3U)) >> 2) + unaff_EDI;
    puVar3 = operator_new(iVar4 + 0xd);
    *puVar3 = 1;
    *(undefined1 *)((int)puVar3 + unaff_EDI + 0xc) = 0;
    puVar3[2] = iVar4;
    puVar3[1] = unaff_EDI;
    *unaff_EBX = (int)(puVar3 + 3);
  }
  return;
}



// ---- FUN_10001150 @ 10001150 ----

void FUN_10001150(void)

{
  LONG LVar1;
  int *unaff_ESI;
  
  if ((undefined1 *)*unaff_ESI != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)((undefined1 *)*unaff_ESI + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*unaff_ESI + -0xc));
    }
  }
  return;
}



// ---- FUN_10001180 @ 10001180 ----

void FUN_10001180(LPCSTR param_1)

{
  size_t _Size;
  undefined4 *puVar1;
  undefined4 *unaff_EBX;
  int iVar2;
  
  *unaff_EBX = PTR_DAT_100085c0;
  if ((param_1 != (LPCSTR)0x0) && ((uint)param_1 >> 0x10 != 0)) {
    _Size = lstrlenA(param_1);
    if (_Size != 0) {
      iVar2 = ((int)(_Size + ((int)_Size >> 0x1f & 3U)) >> 2) + _Size;
      puVar1 = operator_new(iVar2 + 0xd);
      *puVar1 = 1;
      *(undefined1 *)((int)puVar1 + _Size + 0xc) = 0;
      puVar1[1] = _Size;
      puVar1[2] = iVar2;
      *unaff_EBX = puVar1 + 3;
      memcpy(puVar1 + 3,param_1,_Size);
      return;
    }
    return;
  }
  return;
}



// ---- FUN_10001200 @ 10001200 ----

int * __fastcall FUN_10001200(int *param_1)

{
  undefined1 *_Src;
  undefined1 *puVar1;
  size_t _Size;
  int iVar2;
  int *in_EAX;
  LONG LVar3;
  
  _Src = (undefined1 *)*in_EAX;
  puVar1 = (undefined1 *)*param_1;
  if (puVar1 != _Src) {
    if (((*(int *)(puVar1 + -0xc) < 0) && (puVar1 != &DAT_100085b8)) || (*(int *)(_Src + -0xc) < 0))
    {
      _Size = *(size_t *)(_Src + -8);
      FUN_100010d0();
      memcpy((void *)*param_1,_Src,_Size);
      *(size_t *)(*param_1 + -8) = _Size;
      *(undefined1 *)(_Size + *param_1) = 0;
      return param_1;
    }
    if (puVar1 != &DAT_100085b8) {
      LVar3 = InterlockedDecrement((LONG *)(puVar1 + -0xc));
      if (LVar3 < 1) {
        operator_delete((void *)(*param_1 + -0xc));
      }
      *param_1 = (int)PTR_DAT_100085c0;
    }
    iVar2 = *in_EAX;
    *param_1 = iVar2;
    InterlockedIncrement((LONG *)(iVar2 + -0xc));
  }
  return param_1;
}



// ---- FUN_10001290 @ 10001290 ----

void * FUN_10001290(void)

{
  LONG *lpAddend;
  int iVar1;
  int in_EAX;
  undefined4 *puVar2;
  LONG LVar3;
  void *_Src;
  int *unaff_EBX;
  int iVar4;
  
  _Src = (void *)*unaff_EBX;
  lpAddend = (LONG *)((int)_Src + -0xc);
  if ((1 < *(int *)((int)_Src + -0xc)) || (*(int *)((int)_Src + -4) < in_EAX)) {
    iVar1 = *(int *)((int)_Src + -8);
    if (in_EAX < iVar1) {
      in_EAX = iVar1;
    }
    puVar2 = (undefined4 *)PTR_DAT_100085c0;
    if (in_EAX != 0) {
      iVar4 = ((int)(in_EAX + (in_EAX >> 0x1f & 3U)) >> 2) + in_EAX;
      puVar2 = operator_new(iVar4 + 0xd);
      *puVar2 = 1;
      *(undefined1 *)((int)puVar2 + in_EAX + 0xc) = 0;
      puVar2[2] = iVar4;
      puVar2[1] = in_EAX;
      puVar2 = puVar2 + 3;
    }
    *unaff_EBX = (int)puVar2;
    memcpy((void *)*unaff_EBX,_Src,iVar1 + 1);
    *(int *)(*unaff_EBX + -8) = iVar1;
    if ((lpAddend != (LONG *)&DAT_100085ac) && (LVar3 = InterlockedDecrement(lpAddend), LVar3 < 1))
    {
      operator_delete(lpAddend);
    }
    _Src = (void *)*unaff_EBX;
  }
  return _Src;
}



// ---- FUN_10001340 @ 10001340 ----

void __fastcall FUN_10001340(int *param_1)

{
  int in_EAX;
  
  FUN_10001030();
  if (in_EAX == -1) {
    in_EAX = lstrlenA((LPCSTR)*param_1);
  }
  *(int *)(*param_1 + -8) = in_EAX;
  *(undefined1 *)(in_EAX + *param_1) = 0;
  return;
}



// ---- FUN_10001370 @ 10001370 ----

void FUN_10001370(int *param_1,uchar *param_2,int *param_3,int param_4)

{
  short sVar1;
  short *psVar2;
  int *_Args;
  uchar uVar3;
  uchar *_Str;
  int iVar4;
  short *psVar5;
  uint uVar6;
  code *pcVar7;
  int *piVar8;
  int *piVar9;
  int *piVar10;
  bool bVar11;
  int local_8;
  
  _Args = param_3;
  uVar3 = *param_2;
  _Str = param_2;
  piVar9 = param_3;
  pcVar7 = _mbsinc_exref;
  do {
    if (uVar3 == '\0') {
      FUN_10001290();
      iVar4 = _vsnprintf((char *)(*param_1 + param_4),(*(int *)(*param_1 + -4) - param_4) + 1,
                         (char *)param_2,(va_list)_Args);
      if (-1 < iVar4) {
        param_4 = param_4 + iVar4;
      }
      FUN_10001030();
      if (param_4 == -1) {
        param_4 = lstrlenA((LPCSTR)*param_1);
      }
      *(int *)(*param_1 + -8) = param_4;
      *(undefined1 *)(param_4 + *param_1) = 0;
      return;
    }
    if (*_Str != '%') {
LAB_10001689:
      _mbclen(_Str);
      goto LAB_10001697;
    }
    _Str = (uchar *)(*pcVar7)(_Str);
    uVar3 = *_Str;
    if (uVar3 == '%') goto LAB_10001689;
    param_3 = (int *)0x0;
    if (uVar3 == '\0') {
LAB_10001404:
      param_3 = (int *)atoi((char *)_Str);
      uVar3 = *_Str;
      while ((piVar8 = piVar9, uVar3 != '\0' && (iVar4 = _ismbcdigit((int)(char)uVar3), iVar4 != 0))
            ) {
        _Str = (uchar *)(*pcVar7)(_Str);
        uVar3 = *_Str;
      }
    }
    else {
      do {
        if (uVar3 != '#') {
          if (uVar3 == '*') {
            param_3 = (int *)*piVar9;
            piVar9 = piVar9 + 1;
          }
          else if ((((uVar3 != '-') && (uVar3 != '+')) && (uVar3 != '0')) && (uVar3 != ' ')) break;
        }
        _Str = (uchar *)(*pcVar7)(_Str);
        uVar3 = *_Str;
      } while (uVar3 != '\0');
      piVar8 = piVar9;
      if (param_3 == (int *)0x0) goto LAB_10001404;
    }
    local_8 = 0;
    if (*_Str == '.') {
      _Str = (uchar *)(*pcVar7)(_Str);
      if (*_Str == '*') {
        local_8 = *piVar8;
        piVar8 = piVar8 + 1;
        _Str = (uchar *)(*pcVar7)();
      }
      else {
        local_8 = atoi((char *)_Str);
        uVar3 = *_Str;
        while ((uVar3 != '\0' && (iVar4 = _ismbcdigit((int)(char)uVar3), iVar4 != 0))) {
          _Str = (uchar *)(*pcVar7)(_Str);
          uVar3 = *_Str;
        }
      }
    }
    uVar6 = 0;
    switch(*_Str) {
    case 'F':
    case 'L':
    case 'N':
      break;
    default:
      goto switchD_100014b3_caseD_47;
    case 'h':
      uVar6 = 0x10000;
      break;
    case 'l':
      uVar6 = 0x20000;
    }
    _Str = _mbsinc(_Str);
switchD_100014b3_caseD_47:
    uVar6 = (int)(char)*_Str | uVar6;
    piVar9 = piVar8;
    if ((int)uVar6 < 0x10064) {
      if (uVar6 == 0x10063) goto LAB_10001554;
      if ((int)uVar6 < 0x74) {
        if (uVar6 != 0x73) {
          if (uVar6 == 0x43) goto LAB_10001554;
          if (uVar6 != 0x53) {
            bVar11 = uVar6 == 99;
            goto LAB_100014fc;
          }
          psVar2 = (short *)*piVar8;
          psVar5 = psVar2;
          if (psVar2 != (short *)0x0) {
            do {
              sVar1 = *psVar5;
              psVar5 = psVar5 + 1;
            } while (sVar1 != 0);
            goto LAB_1000153b;
          }
          piVar10 = (int *)0x6;
          goto LAB_100015de;
        }
      }
      else {
        if (uVar6 == 0x10043) goto LAB_10001554;
        if (uVar6 != 0x10053) goto LAB_100014fe;
      }
LAB_10001561:
      if ((LPCSTR)*piVar8 == (LPCSTR)0x0) {
        piVar10 = (int *)0x6;
      }
      else {
        piVar10 = (int *)lstrlenA((LPCSTR)*piVar8);
LAB_1000153f:
        if ((int)piVar10 < 1) {
          piVar10 = (int *)0x1;
        }
        else {
          piVar9 = piVar8 + 1;
          if (piVar10 == (int *)0x0) goto LAB_100014fe;
        }
      }
LAB_100015de:
      piVar9 = piVar8 + 1;
      if ((int)piVar10 <= (int)param_3) {
        piVar10 = param_3;
      }
      if ((local_8 == 0) || (pcVar7 = _mbsinc_exref, (int)piVar10 < local_8))
      goto switchD_10001514_caseD_48;
      goto LAB_10001697;
    }
    if ((int)uVar6 < 0x20054) {
      if (uVar6 == 0x20053) {
LAB_100015b4:
        psVar2 = (short *)*piVar8;
        psVar5 = psVar2;
        if (psVar2 != (short *)0x0) {
          do {
            sVar1 = *psVar5;
            psVar5 = psVar5 + 1;
          } while (sVar1 != 0);
LAB_1000153b:
          piVar10 = (int *)((int)psVar5 - (int)(psVar2 + 1) >> 1);
          goto LAB_1000153f;
        }
        piVar10 = (int *)0x6;
        goto LAB_100015de;
      }
      if (uVar6 == 0x10073) goto LAB_10001561;
      bVar11 = uVar6 == 0x20043;
LAB_100014fc:
      if (!bVar11) goto LAB_100014fe;
LAB_10001554:
      piVar10 = (int *)0x2;
      goto LAB_100015de;
    }
    if (uVar6 == 0x20063) goto LAB_10001554;
    if (uVar6 == 0x20073) goto LAB_100015b4;
LAB_100014fe:
    pcVar7 = _mbsinc_exref;
    switch(*_Str) {
    case 'G':
    case 'e':
    case 'f':
    case 'g':
      piVar9 = piVar9 + 2;
      if (0x7f < local_8 + (int)param_3) goto LAB_10001697;
      break;
    case 'X':
    case 'd':
    case 'i':
    case 'o':
    case 'u':
    case 'x':
      goto joined_r0x1000166a;
    case 'n':
      piVar9 = piVar9 + 1;
      break;
    case 'p':
joined_r0x1000166a:
      piVar9 = piVar9 + 1;
      if ((int)param_3 + local_8 < 0x20) break;
      goto LAB_10001697;
    }
switchD_10001514_caseD_48:
    pcVar7 = _mbsinc_exref;
LAB_10001697:
    _Str = (uchar *)(*pcVar7)(_Str);
    uVar3 = *_Str;
  } while( true );
}



// ---- FUN_10001790 @ 10001790 ----

/* WARNING: Globals starting with '_' overlap smaller symbols at the same address */

int * FUN_10001790(void)

{
  char cVar1;
  int *piVar2;
  
  piVar2 = operator_new(0x110);
  if (piVar2 != (int *)0x0) {
    *piVar2 = -1;
    piVar2[1] = 0;
    piVar2[0xd] = 0;
    piVar2[0xf] = 0;
    piVar2[0xe] = 0;
    piVar2[0x10] = 0;
    piVar2[0x12] = 0;
    piVar2[0x11] = 0;
    piVar2[0x13] = 0;
    piVar2[0x14] = 0;
    *(undefined1 *)(piVar2 + 0x15) = 0;
    piVar2[0x16] = 1;
    piVar2[0x17] = (int)(piVar2 + 0x1a);
    piVar2[0x18] = 0;
    piVar2[0x19] = 10;
    *(undefined2 *)(piVar2 + 0x42) = 0;
    piVar2[0x43] = 0;
    cVar1 = FUN_100029c0(piVar2);
    if (cVar1 != '\0') {
      _DAT_10008948 = piVar2[1];
      FUN_10004170(piVar2);
      return piVar2;
    }
  }
  _DAT_10008948 = 0;
  return (int *)0x0;
}



// ---- InitNMC @ 10001820 ----

void InitNMC(void)

{
  int *piVar1;
  LPARAM LVar2;
  DWORD DVar3;
  int iVar4;
  undefined4 uVar5;
  void *pvVar6;
  uint local_28;
  tagMSG local_24;
  
                    /* 0x1820  4  InitNMC */
  DAT_10008938 = GetCurrentThreadId();
  local_28 = 3000;
  DAT_1000892c = FUN_10001790();
LAB_10001849:
  do {
    while( true ) {
      if (((DAT_1000892c == (int *)0x0) && (local_28 == 0xffffffff)) ||
         (DVar3 = MsgWaitForMultipleObjects
                            ((DAT_1000892c != (int *)0x0) + 1,(HANDLE *)&DAT_10008944,0,local_28,8),
         piVar1 = DAT_1000892c, DVar3 == 0xffffffff)) {
        return;
      }
      if (DVar3 == 1) break;
      if (DVar3 == 2) {
        iVar4 = PeekMessageA(&local_24,(HWND)0x0,0,0,1);
        while (iVar4 != 0) {
          TranslateMessage(&local_24);
          LVar2 = local_24.lParam;
          if (DAT_1000892c != (int *)0x0) {
            if ((void *)local_24.lParam == (void *)0x0) {
              if (local_24.message == 0x8005) {
                FUN_100046f0(1,0);
              }
              else if (local_24.message == 0x8007) {
                FUN_10004820();
              }
            }
            else if (local_24.message == 0x8006) {
              piVar1 = (int *)(local_24.lParam + 4);
              if (*(int *)(local_24.lParam + 4) == 0) {
                uVar5 = *(undefined4 *)(*(int *)(local_24.lParam + 8) + 4);
              }
              else {
                uVar5 = *(undefined4 *)(*(int *)(local_24.lParam + 4) + 4);
              }
              FUN_10004410(uVar5,(undefined4 *)(local_24.lParam + 0xc),
                           (undefined4 *)(local_24.lParam + 0x10),
                           (undefined4 *)(local_24.lParam + 0x14));
              pvVar6 = (void *)*piVar1;
              if (pvVar6 == (void *)0x0) {
                pvVar6 = *(void **)(LVar2 + 8);
                if (pvVar6 != (void *)0x0) {
                  FUN_10001c40((int)pvVar6);
                  goto LAB_1000195a;
                }
              }
              else {
                FUN_10001ac0((int)pvVar6);
LAB_1000195a:
                operator_delete(pvVar6);
              }
              FUN_10001df0(LVar2);
              operator_delete((void *)LVar2);
            }
            else if (local_24.message == 0x8008) {
              FUN_100048f0();
            }
            else {
              if (local_24.message == 0x8003) {
                FUN_100046f0(*(int *)(local_24.lParam + 8),local_24.lParam);
              }
              if (local_24.message == 0x8002) {
                FUN_10004580((void *)(LVar2 + 0x10),*(undefined4 *)(LVar2 + 4),*(int *)(LVar2 + 8),
                             LVar2);
              }
            }
          }
          DispatchMessageA(&local_24);
          iVar4 = PeekMessageA(&local_24,(HWND)0x0,0,0,1);
        }
      }
      else if (((DVar3 == 0x102) && (DAT_1000892c == (int *)0x0)) &&
              (DAT_1000892c = FUN_10001790(), DAT_1000892c == (int *)0x0)) {
        if (local_28 < 30000) {
          local_28 = local_28 * 2;
        }
        else {
          local_28 = 60000;
        }
      }
    }
    if (DAT_1000892c == (int *)0x0) {
LAB_10001a3d:
      DAT_1000892c = (int *)0x0;
      local_28 = 1000;
      iVar4 = PeekMessageA(&local_24,(HWND)0x0,0,0,1);
      while (iVar4 != 0) {
        TranslateMessage(&local_24);
        DispatchMessageA(&local_24);
        iVar4 = PeekMessageA(&local_24,(HWND)0x0,0,0,1);
      }
      goto LAB_10001849;
    }
    if (*DAT_1000892c == -1) {
LAB_10001a2d:
      FUN_10002940();
      operator_delete(piVar1);
      goto LAB_10001a3d;
    }
    ResetEvent((HANDLE)DAT_1000892c[1]);
    FUN_10002b00();
    FUN_10002d40();
    if (*piVar1 == -1) goto LAB_10001a2d;
    DAT_10008930 = 1;
  } while( true );
}



// ---- FUN_10001ac0 @ 10001ac0 ----

void FUN_10001ac0(int param_1)

{
  LONG LVar1;
  void *local_c;
  undefined1 *puStack_8;
  int local_4;
  
  puStack_8 = &LAB_10005762;
  local_c = ExceptionList;
  ExceptionList = &local_c;
  local_4 = 5;
  if (*(undefined1 **)(param_1 + 0x24) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x24) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x24) + -0xc));
    }
  }
  local_4._0_1_ = 4;
  if (*(undefined1 **)(param_1 + 0x20) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x20) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x20) + -0xc));
    }
  }
  local_4._0_1_ = 3;
  if (*(undefined1 **)(param_1 + 0x1c) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x1c) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x1c) + -0xc));
    }
  }
  local_4._0_1_ = 2;
  if (*(undefined1 **)(param_1 + 0x18) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x18) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x18) + -0xc));
    }
  }
  local_4._0_1_ = 1;
  if (*(undefined1 **)(param_1 + 0x14) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x14) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x14) + -0xc));
    }
  }
  local_4 = (uint)local_4._1_3_ << 8;
  if (*(undefined1 **)(param_1 + 0x10) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x10) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x10) + -0xc));
    }
  }
  local_4 = 0xffffffff;
  if (*(undefined1 **)(param_1 + 0xc) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0xc) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0xc) + -0xc));
    }
  }
  ExceptionList = local_c;
  return;
}



// ---- FUN_10001c40 @ 10001c40 ----

void FUN_10001c40(int param_1)

{
  LONG LVar1;
  void *local_c;
  undefined1 *puStack_8;
  int local_4;
  
  puStack_8 = &LAB_100056fd;
  local_c = ExceptionList;
  ExceptionList = &local_c;
  local_4 = 6;
  if (*(undefined1 **)(param_1 + 0x28) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x28) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x28) + -0xc));
    }
  }
  local_4._0_1_ = 5;
  if (*(undefined1 **)(param_1 + 0x24) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x24) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x24) + -0xc));
    }
  }
  local_4._0_1_ = 4;
  if (*(undefined1 **)(param_1 + 0x20) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x20) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x20) + -0xc));
    }
  }
  local_4._0_1_ = 3;
  if (*(undefined1 **)(param_1 + 0x1c) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x1c) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x1c) + -0xc));
    }
  }
  local_4._0_1_ = 2;
  if (*(undefined1 **)(param_1 + 0x18) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x18) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x18) + -0xc));
    }
  }
  local_4._0_1_ = 1;
  if (*(undefined1 **)(param_1 + 0x14) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x14) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x14) + -0xc));
    }
  }
  local_4 = (uint)local_4._1_3_ << 8;
  if (*(undefined1 **)(param_1 + 0x10) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x10) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x10) + -0xc));
    }
  }
  local_4 = 0xffffffff;
  if (*(undefined1 **)(param_1 + 0xc) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0xc) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0xc) + -0xc));
    }
  }
  ExceptionList = local_c;
  return;
}



// ---- FUN_10001df0 @ 10001df0 ----

void FUN_10001df0(int param_1)

{
  LONG LVar1;
  void *local_c;
  undefined1 *puStack_8;
  uint local_4;
  
  puStack_8 = &LAB_10005686;
  local_c = ExceptionList;
  ExceptionList = &local_c;
  local_4 = 1;
  if (*(undefined1 **)(param_1 + 0x14) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x14) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x14) + -0xc));
    }
  }
  local_4 = local_4 & 0xffffff00;
  if (*(undefined1 **)(param_1 + 0x10) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x10) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x10) + -0xc));
    }
  }
  local_4 = 0xffffffff;
  if (*(undefined1 **)(param_1 + 0xc) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0xc) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0xc) + -0xc));
    }
  }
  ExceptionList = local_c;
  return;
}



// ---- InitNMC2 @ 10001ec0 ----

void __cdecl InitNMC2(undefined4 param_1)

{
                    /* 0x1ec0  5  InitNMC2 */
  DAT_10008928 = param_1;
  InitNMC();
  return;
}



// ---- SetMainWnd @ 10001ed0 ----

void __cdecl SetMainWnd(undefined4 param_1)

{
                    /* 0x1ed0  10  SetMainWnd */
  DAT_1000893c = param_1;
  return;
}



// ---- SendDwnlResult @ 10001ee0 ----

undefined4 __cdecl
SendDwnlResult(undefined4 param_1,undefined4 param_2,LPCSTR param_3,LPCSTR param_4,LPCSTR param_5)

{
  undefined4 *lParam;
  
                    /* 0x1ee0  6  SendDwnlResult */
  if (DAT_1000892c == 0) {
    return 6;
  }
  if (DAT_10008934 != 0) {
    return 5;
  }
  if (DAT_10008930 == 0) {
    Sleep(500);
    DAT_10008930 = 1;
  }
  if (DAT_10008938 == 0) {
    return 0x12;
  }
  lParam = operator_new(0x18);
  if (lParam != (undefined4 *)0x0) {
    lParam[3] = PTR_DAT_100085c0;
    lParam[4] = PTR_DAT_100085c0;
    lParam[5] = PTR_DAT_100085c0;
    *lParam = param_2;
    lParam[1] = param_1;
    lParam[2] = 0;
    FUN_10001180(param_3);
    FUN_10001200(lParam + 3);
    FUN_10001150();
    FUN_10001180(param_4);
    FUN_10001200(lParam + 4);
    FUN_10001150();
    FUN_10001180(param_5);
    FUN_10001200(lParam + 5);
    FUN_10001150();
    PostThreadMessageA(DAT_10008938,0x8006,0,(LPARAM)lParam);
    return 0;
  }
  return 8;
}



// ---- SendDwnlResult2 @ 10002000 ----

undefined4 __cdecl
SendDwnlResult2(undefined4 param_1,undefined4 param_2,LPCSTR param_3,LPCSTR param_4,LPCSTR param_5)

{
  undefined4 *lParam;
  
                    /* 0x2000  7  SendDwnlResult2 */
  if (DAT_1000892c == 0) {
    return 6;
  }
  if (DAT_10008934 != 0) {
    return 5;
  }
  if (DAT_10008930 == 0) {
    Sleep(500);
    DAT_10008930 = 1;
  }
  if (DAT_10008938 == 0) {
    return 0x12;
  }
  lParam = operator_new(0x18);
  if (lParam != (undefined4 *)0x0) {
    lParam[3] = PTR_DAT_100085c0;
    lParam[4] = PTR_DAT_100085c0;
    lParam[5] = PTR_DAT_100085c0;
    *lParam = param_2;
    lParam[2] = param_1;
    lParam[1] = 0;
    FUN_10001180(param_3);
    FUN_10001200(lParam + 3);
    FUN_10001150();
    FUN_10001180(param_4);
    FUN_10001200(lParam + 4);
    FUN_10001150();
    FUN_10001180(param_5);
    FUN_10001200(lParam + 5);
    FUN_10001150();
    PostThreadMessageA(DAT_10008938,0x8006,0,(LPARAM)lParam);
    return 0;
  }
  return 8;
}



// ---- SendRepairH @ 10002120 ----

void SendRepairH(void)

{
                    /* 0x2120  8  SendRepairH */
  if ((DAT_1000892c != 0) && (DAT_10008934 == 0)) {
    if (DAT_10008930 == 0) {
      Sleep(500);
      DAT_10008930 = 1;
    }
    if (DAT_10008938 != 0) {
      PostThreadMessageA(DAT_10008938,0x8007,0,0);
    }
  }
  return;
}



// ---- SendUnblock @ 10002170 ----

void __cdecl SendUnblock(LPARAM param_1)

{
                    /* 0x2170  9  SendUnblock */
  if ((DAT_1000892c != 0) && (DAT_10008934 == 0)) {
    if (DAT_10008930 == 0) {
      Sleep(500);
      DAT_10008930 = 1;
    }
    if (DAT_10008938 != 0) {
      PostThreadMessageA(DAT_10008938,0x8008,0,param_1);
    }
  }
  return;
}



// ---- GetActiveOnWakeup @ 100021c0 ----

undefined4 GetActiveOnWakeup(void)

{
                    /* 0x21c0  2  GetActiveOnWakeup */
  if (DAT_1000892c == 0) {
    return 6;
  }
  if (DAT_10008934 != 0) {
    return 5;
  }
  if (DAT_10008930 == 0) {
    Sleep(500);
    DAT_10008930 = 1;
  }
  if (DAT_10008938 == 0) {
    return 0x12;
  }
  PostThreadMessageA(DAT_10008938,0x8005,0,0);
  return 0;
}



// ---- GetCks @ 10002220 ----

int __cdecl GetCks(LPCSTR param_1,undefined4 param_2,int *param_3,undefined4 *param_4)

{
  char cVar1;
  DWORD DVar2;
  undefined4 *lParam;
  HANDLE pvVar3;
  char *pcVar4;
  char *pcVar5;
  int iVar6;
  
                    /* 0x2220  3  GetCks */
  if (DAT_1000892c == 0) {
    return 6;
  }
  if (DAT_10008934 != 0) {
    return 5;
  }
  if (DAT_10008930 == 0) {
    Sleep(500);
    DAT_10008930 = 1;
  }
  if (DAT_10008938 == 0) {
    return 0x12;
  }
  lParam = operator_new(0x1c);
  if (lParam == (undefined4 *)0x0) {
    return 7;
  }
  lParam[3] = PTR_DAT_100085c0;
  lParam[4] = PTR_DAT_100085c0;
  pvVar3 = CreateEventA((LPSECURITY_ATTRIBUTES)0x0,1,0,(LPCSTR)0x0);
  *lParam = pvVar3;
  if (pvVar3 == (HANDLE)0x0) {
    FUN_10002400((int)lParam);
    operator_delete(lParam);
    return 0x13;
  }
  FUN_10001180(param_1);
  FUN_10001200(lParam + 4);
  FUN_10001150();
  lParam[2] = param_2;
  lParam[5] = 0xffffffff;
  lParam[6] = 0;
  DVar2 = DAT_10008938;
  if (*param_3 == 0) {
    lParam[1] = 0;
    PostThreadMessageA(DVar2,0x8003,0,(LPARAM)lParam);
    WaitForSingleObject((HANDLE)*lParam,1000);
    ResetEvent((HANDLE)*lParam);
    iVar6 = lParam[5];
    if (iVar6 == 0) {
      if (lParam[1] != 0) {
        *param_3 = lParam[1];
        goto LAB_1000236f;
      }
      iVar6 = 0xd;
    }
    if (lParam[6] == 0) {
      return iVar6;
    }
    pvVar3 = (HANDLE)*lParam;
  }
  else {
    lParam[1] = *param_3;
LAB_1000236f:
    DVar2 = DAT_10008938;
    lParam[6] = 0;
    PostThreadMessageA(DVar2,0x8002,0,(LPARAM)lParam);
    WaitForSingleObject((HANDLE)*lParam,1000);
    ResetEvent((HANDLE)*lParam);
    if (*(int *)(lParam[3] + -8) == 0) {
      iVar6 = lParam[5];
    }
    else {
      pcVar4 = operator_new__(*(int *)(lParam[3] + -8) + 1);
      *param_4 = pcVar4;
      if (pcVar4 == (char *)0x0) {
        iVar6 = 9;
      }
      else {
        pcVar5 = (char *)lParam[3];
        do {
          cVar1 = *pcVar5;
          *pcVar4 = cVar1;
          pcVar5 = pcVar5 + 1;
          pcVar4 = pcVar4 + 1;
        } while (cVar1 != '\0');
        iVar6 = 0;
      }
    }
    if (lParam[6] == 0) {
      return iVar6;
    }
    pvVar3 = (HANDLE)*lParam;
  }
  CloseHandle(pvVar3);
  FUN_10002400((int)lParam);
  operator_delete(lParam);
  return iVar6;
}



// ---- FUN_10002400 @ 10002400 ----

void FUN_10002400(int param_1)

{
  LONG LVar1;
  void *local_c;
  undefined1 *puStack_8;
  undefined4 local_4;
  
  puStack_8 = &LAB_1000564b;
  local_c = ExceptionList;
  ExceptionList = &local_c;
  local_4 = 0;
  if (*(undefined1 **)(param_1 + 0x10) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0x10) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0x10) + -0xc));
    }
  }
  local_4 = 0xffffffff;
  if (*(undefined1 **)(param_1 + 0xc) != &DAT_100085b8) {
    LVar1 = InterlockedDecrement((LONG *)(*(undefined1 **)(param_1 + 0xc) + -0xc));
    if (LVar1 < 1) {
      operator_delete((void *)(*(int *)(param_1 + 0xc) + -0xc));
    }
  }
  ExceptionList = local_c;
  return;
}



// ---- FUN_100024b0 @ 100024b0 ----

/* WARNING: Globals starting with '_' overlap smaller symbols at the same address */

undefined4 FUN_100024b0(undefined4 param_1,int param_2)

{
  if (param_2 == 1) {
    DAT_10008924 = GetModuleHandleA("MSVCRT.DLL");
    if (DAT_10008924 != (HMODULE)0x0) {
      DAT_1000891c = GetProcAddress(DAT_10008924,"_vsnprintf_l");
      _DAT_10008920 = GetProcAddress(DAT_10008924,"_sscanf_l");
    }
  }
  return 1;
}



// ---- FUN_10002500 @ 10002500 ----

void FUN_10002500(void)

{
  undefined1 *in_EAX;
  
  *in_EAX = 0;
  in_EAX[0x10] = 0;
  *(undefined4 *)(in_EAX + 8) = 0;
  *(undefined4 *)(in_EAX + 0xc) = 0;
  *(undefined4 *)(in_EAX + 0x18) = 0;
  *(undefined4 *)(in_EAX + 0x1c) = 0;
  in_EAX[0x20] = 0;
  *(undefined4 *)(in_EAX + 0x28) = 0;
  *(undefined4 *)(in_EAX + 0x2c) = 0;
  in_EAX[0x30] = 0;
  *(undefined4 *)(in_EAX + 0x38) = 0;
  *(undefined4 *)(in_EAX + 0x3c) = 0;
  in_EAX[0x40] = 0;
  *(undefined4 *)(in_EAX + 0x48) = 0;
  *(undefined4 *)(in_EAX + 0x4c) = 0;
  in_EAX[0x50] = 0;
  *(undefined4 *)(in_EAX + 0x58) = 0;
  *(undefined4 *)(in_EAX + 0x5c) = 0;
  in_EAX[0x60] = 0;
  *(undefined4 *)(in_EAX + 0x68) = 0;
  *(undefined4 *)(in_EAX + 0x6c) = 0;
  in_EAX[0x70] = 0;
  *(undefined4 *)(in_EAX + 0x78) = 0;
  *(undefined4 *)(in_EAX + 0x7c) = 0;
  in_EAX[0x80] = 0;
  *(undefined4 *)(in_EAX + 0x88) = 0;
  *(undefined4 *)(in_EAX + 0x8c) = 0;
  in_EAX[0x90] = 0;
  *(undefined4 *)(in_EAX + 0x98) = 0;
  *(undefined4 *)(in_EAX + 0x9c) = 0;
  return;
}



// ---- FUN_10002580 @ 10002580 ----

undefined4 __fastcall FUN_10002580(undefined4 param_1,int *param_2,uchar *param_3)

{
  undefined4 uVar1;
  
  uVar1 = FUN_10001370(param_2,param_3,(int *)&stack0x00000008,0);
  return CONCAT31((int3)((uint)uVar1 >> 8),1);
}



// ---- FUN_100025a0 @ 100025a0 ----

undefined4 __cdecl FUN_100025a0(uchar *param_1)

{
  int *in_EAX;
  undefined4 uVar1;
  
  uVar1 = FUN_10001370(in_EAX,param_1,(int *)&stack0x00000008,*(int *)(*in_EAX + -8));
  return CONCAT31((int3)((uint)uVar1 >> 8),1);
}



// ---- FUN_100025c0 @ 100025c0 ----

void FUN_100025c0(void)

{
  undefined4 *in_EAX;
  
  if ((void *)*in_EAX != (void *)0x0) {
    free((void *)*in_EAX);
  }
  return;
}



// ---- FUN_100025d0 @ 100025d0 ----

uint __fastcall FUN_100025d0(undefined4 param_1,uint param_2,byte *param_3,uint param_4)

{
  byte bVar1;
  undefined4 uVar2;
  int iVar3;
  byte *in_EAX;
  uint uVar4;
  uint uVar5;
  uint uVar6;
  int iVar7;
  int iVar8;
  byte *pbVar9;
  uint local_10;
  byte local_8;
  
  if ((((param_3 == (byte *)0x0) || (in_EAX == (byte *)0x0)) || (param_2 == 0)) || (param_4 == 0)) {
    return 0;
  }
  uVar4 = 0;
  uVar6 = 0;
  if (param_2 != 0) {
    while (((bVar1 = *in_EAX, bVar1 == 0x20 || (bVar1 == 9)) || ((bVar1 == 0xd || (bVar1 == 10)))))
    {
      in_EAX = in_EAX + 1;
      param_2 = param_2 - 1;
      if (param_2 == 0) {
        return 0;
      }
    }
    if (param_2 != 0) {
      do {
        if (in_EAX[uVar4] == 0x3d) {
          uVar6 = uVar6 + 1;
        }
        else if (0x3f < *(int *)(&DAT_100069c8 + (uint)in_EAX[uVar4] * 4)) break;
        uVar4 = uVar4 + 1;
      } while (uVar4 < param_2);
      if (uVar4 != 0) {
        uVar5 = uVar4 & 3;
        local_10 = (uVar4 >> 2) * 3;
        if (uVar5 == 2) {
          local_10 = local_10 + 1;
        }
        else if (uVar5 == 3) {
          local_10 = local_10 + 2;
        }
        if (local_10 < uVar6) {
          local_10 = 0;
        }
        else {
          local_10 = local_10 - uVar6;
        }
        if (local_10 < param_4) {
          pbVar9 = param_3;
          if ((uVar4 & 0xfffffffc) != 0) {
            iVar7 = ((uVar4 & 0xfffffffc) - 1 >> 2) + 1;
            do {
              iVar8 = *(int *)(&DAT_100069c8 + (uint)in_EAX[1] * 4);
              uVar2 = *(undefined4 *)(&DAT_100069c8 + (uint)in_EAX[3] * 4);
              iVar3 = *(int *)(&DAT_100069c8 + (uint)in_EAX[2] * 4);
              *pbVar9 = (&DAT_100069c8)[(uint)*in_EAX * 4] * '\x04' | (byte)(iVar8 >> 4);
              local_8 = (byte)uVar2;
              pbVar9[1] = (byte)(iVar3 >> 2) | (char)iVar8 << 4;
              pbVar9[2] = (char)iVar3 << 6 | local_8;
              pbVar9 = pbVar9 + 3;
              in_EAX = in_EAX + 4;
              iVar7 = iVar7 + -1;
            } while (iVar7 != 0);
          }
          if ((uVar5 != 0) && (uVar6 == 0)) {
            if (uVar5 < 2) {
              iVar7 = 0;
            }
            else {
              iVar7 = *(int *)(&DAT_100069c8 + (uint)in_EAX[1] * 4);
            }
            if (uVar5 < 3) {
              iVar8 = 0;
            }
            else {
              iVar8 = *(int *)(&DAT_100069c8 + (uint)in_EAX[2] * 4);
            }
            if (1 < uVar5) {
              *pbVar9 = (byte)(iVar7 >> 4) |
                        (char)*(undefined4 *)(&DAT_100069c8 + (uint)*in_EAX * 4) * '\x04';
              pbVar9 = pbVar9 + 1;
            }
            if (uVar5 == 3) {
              *pbVar9 = (byte)(iVar8 >> 2) | (char)iVar7 << 4;
            }
          }
          param_3[local_10] = 0;
          return local_10;
        }
      }
    }
  }
  return 0;
}



// ---- FUN_100027b0 @ 100027b0 ----

bool __thiscall FUN_100027b0(void *this,char *param_1)

{
  void *pvVar1;
  int iVar2;
  uint uVar3;
  int iVar4;
  char *_Dest;
  int *unaff_ESI;
  
  if ((char)unaff_ESI[3] != '\0') {
    return false;
  }
  uVar3 = (int)this + unaff_ESI[1] + 1;
  if ((uint)unaff_ESI[2] < uVar3) {
    iVar2 = uVar3 + 0x10 + (uVar3 >> 2);
    pvVar1 = realloc((void *)*unaff_ESI,iVar2 + 1);
    if (pvVar1 == (void *)0x0) {
      _Dest = (char *)0x0;
      goto LAB_100027f5;
    }
    *unaff_ESI = (int)pvVar1;
    unaff_ESI[2] = iVar2;
  }
  _Dest = (char *)(*unaff_ESI + unaff_ESI[1]);
LAB_100027f5:
  if (DAT_1000891c == (code *)0x0) {
    iVar2 = _vsnprintf(_Dest,unaff_ESI[2] - unaff_ESI[1],param_1,&stack0x00000008);
  }
  else {
    iVar2 = (*DAT_1000891c)(_Dest,unaff_ESI[2] - unaff_ESI[1],param_1,&PTR_DAT_10008244,
                            &stack0x00000008);
  }
  uVar3 = unaff_ESI[1] + iVar2;
  if ((uint)unaff_ESI[2] < uVar3) {
    iVar4 = uVar3 + 0x10 + (uVar3 >> 2);
    pvVar1 = realloc((void *)*unaff_ESI,iVar4 + 1);
    if (pvVar1 == (void *)0x0) {
      return false;
    }
    *unaff_ESI = (int)pvVar1;
    unaff_ESI[2] = iVar4;
  }
  iVar4 = *unaff_ESI + unaff_ESI[1];
  if (iVar4 != 0) {
    unaff_ESI[1] = unaff_ESI[1] + iVar2;
  }
  return iVar4 != 0;
}



// ---- FUN_10002890 @ 10002890 ----

uint __thiscall FUN_10002890(void *this,void *param_1,size_t param_2)

{
  uint uVar1;
  int iVar2;
  bool bVar3;
  uint in_EAX;
  void *pvVar4;
  int iVar5;
  
  if ((*(char *)((int)this + 0xc) != '\0') || (param_2 == 0)) {
    return in_EAX & 0xffffff00;
  }
  bVar3 = FUN_100027b0((void *)(param_2 + 0x17),",%u=%u:");
  if ((bVar3) && (param_1 != (void *)0x0)) {
    iVar5 = *(int *)((int)this + 4);
    uVar1 = iVar5 + param_2;
    if (uVar1 <= *(uint *)((int)this + 8)) {
LAB_10002901:
      memcpy((void *)(*(int *)this + iVar5),param_1,param_2);
      *(uint *)((int)this + 4) = uVar1;
      return 1;
    }
    iVar2 = uVar1 + 0x10 + (uVar1 >> 2);
    pvVar4 = realloc(*(void **)this,iVar2 + 1);
    if (pvVar4 != (void *)0x0) {
      *(void **)this = pvVar4;
      iVar5 = *(int *)((int)this + 4);
      *(int *)((int)this + 8) = iVar2;
      if (*(int *)this + iVar5 != 0) goto LAB_10002901;
    }
  }
  return 0;
}



// ---- FUN_10002940 @ 10002940 ----

void FUN_10002940(void)

{
  undefined4 *_Memory;
  undefined4 *in_EAX;
  int iVar1;
  bool bVar2;
  
  *(undefined1 *)(in_EAX + 0x15) = 0;
  if ((HANDLE)*in_EAX != (HANDLE)0xffffffff) {
    CloseHandle((HANDLE)*in_EAX);
    *in_EAX = 0xffffffff;
    CloseHandle((HANDLE)in_EAX[1]);
    in_EAX[1] = 0;
  }
  in_EAX[0x18] = 0;
  iVar1 = in_EAX[0x18];
  do {
    bVar2 = iVar1 != 0;
    iVar1 = iVar1 + -1;
  } while (bVar2);
  in_EAX[0x18] = 0xffffffff;
  _Memory = (undefined4 *)in_EAX[0x17];
  if ((_Memory != in_EAX + 0x1a) && (_Memory != (undefined4 *)0x0)) {
    free(_Memory);
  }
  if ((void *)in_EAX[0x10] != (void *)0x0) {
    free((void *)in_EAX[0x10]);
  }
  if ((void *)in_EAX[0xd] != (void *)0x0) {
    free((void *)in_EAX[0xd]);
  }
  return;
}



// ---- FUN_100029c0 @ 100029c0 ----

void FUN_100029c0(int *param_1)

{
  DWORD dwProcessId;
  HANDLE pvVar1;
  int iVar2;
  undefined4 extraout_EDX;
  undefined4 extraout_EDX_00;
  char *pcVar3;
  CHAR *pCVar4;
  DWORD *pSessionId;
  DWORD local_4c;
  CHAR local_48 [27];
  char local_2d [41];
  uint local_4;
  
  local_4 = DAT_1000828c ^ (uint)&local_4c;
  pSessionId = &local_4c;
  local_4c = 0;
  dwProcessId = GetCurrentProcessId();
  ProcessIdToSessionId(dwProcessId,pSessionId);
  pvVar1 = CreateEventA((LPSECURITY_ATTRIBUTES)0x0,1,0,(LPCSTR)0x0);
  param_1[1] = (int)pvVar1;
  if (pvVar1 != (HANDLE)0x0) {
    pcVar3 = "\\\\.\\pipe\\IDMNetworkMonitor.";
    pCVar4 = local_48;
    for (iVar2 = 7; iVar2 != 0; iVar2 = iVar2 + -1) {
      *(undefined4 *)pCVar4 = *(undefined4 *)pcVar3;
      pcVar3 = pcVar3 + 4;
      pCVar4 = pCVar4 + 4;
    }
    local_2d[1] = '\0';
    local_2d[2] = '\0';
    local_2d[3] = '\0';
    local_2d[4] = '\0';
    local_2d[5] = '\0';
    local_2d[6] = '\0';
    local_2d[7] = '\0';
    local_2d[8] = '\0';
    local_2d[9] = '\0';
    local_2d[10] = '\0';
    local_2d[0xb] = '\0';
    local_2d[0xc] = '\0';
    local_2d[0xd] = '\0';
    local_2d[0xe] = '\0';
    local_2d[0xf] = '\0';
    local_2d[0x10] = '\0';
    local_2d[0x11] = '\0';
    local_2d[0x12] = '\0';
    local_2d[0x13] = '\0';
    local_2d[0x14] = '\0';
    local_2d[0x15] = '\0';
    local_2d[0x16] = '\0';
    local_2d[0x17] = '\0';
    local_2d[0x18] = '\0';
    local_2d[0x19] = '\0';
    local_2d[0x1a] = '\0';
    local_2d[0x1b] = '\0';
    local_2d[0x1c] = '\0';
    local_2d[0x1d] = '\0';
    local_2d[0x1e] = '\0';
    local_2d[0x1f] = '\0';
    local_2d[0x20] = '\0';
    local_2d[0x21] = '\0';
    local_2d[0x22] = '\0';
    local_2d[0x23] = '\0';
    local_2d[0x24] = '\0';
    _itoa(local_4c,local_2d,10);
    pvVar1 = CreateFileA(local_48,0xc0000000,0,(LPSECURITY_ATTRIBUTES)0x0,3,0x60000000,(HANDLE)0x0);
    *param_1 = (int)pvVar1;
    if (pvVar1 != (HANDLE)0xffffffff) {
      param_1[2] = 0;
      param_1[3] = 0;
      param_1[4] = 0;
      param_1[5] = 0;
      param_1[6] = 0;
      param_1[7] = 0;
      param_1[8] = 0;
      param_1[9] = 0;
      param_1[10] = 0;
      param_1[0xb] = 0;
      param_1[0xb] = param_1[1];
      param_1[6] = param_1[1];
      *(undefined1 *)((int)param_1 + 0x31) = 0;
      *(undefined1 *)(param_1 + 0xc) = 0;
      FUN_10002b00();
      if (*param_1 != -1) {
        FUN_10004ab0(local_4 ^ (uint)&local_4c,extraout_EDX);
        return;
      }
    }
  }
  FUN_10002940();
  operator_delete(param_1);
  FUN_10004ab0(local_4 ^ (uint)&local_4c,extraout_EDX_00);
  return;
}



// ---- FUN_10002b00 @ 10002b00 ----

void FUN_10002b00(void)

{
  int iVar1;
  uint uVar2;
  BOOL BVar3;
  uint uVar4;
  void *pvVar5;
  DWORD DVar6;
  LPVOID lpBuffer;
  int *unaff_ESI;
  uint *puVar7;
  DWORD local_8 [2];
  
  if (*unaff_ESI == -1) {
    return;
  }
  do {
    local_8[0] = 0;
    if ((char)unaff_ESI[0xc] == '\0') {
      uVar4 = unaff_ESI[0xe] + 0x1000;
      if ((uint)unaff_ESI[0xf] < uVar4) {
        iVar1 = unaff_ESI[0xe] + 0x1010 + (uVar4 >> 2);
        pvVar5 = realloc((void *)unaff_ESI[0xd],iVar1 + 1);
        if (pvVar5 != (void *)0x0) {
          unaff_ESI[0xd] = (int)pvVar5;
          unaff_ESI[0xf] = iVar1;
          goto LAB_10002b6f;
        }
        lpBuffer = (LPVOID)0x0;
      }
      else {
LAB_10002b6f:
        lpBuffer = (LPVOID)(unaff_ESI[0xe] + unaff_ESI[0xd]);
      }
      BVar3 = ReadFile((HANDLE)*unaff_ESI,lpBuffer,unaff_ESI[0xf] - unaff_ESI[0xe],local_8,
                       (LPOVERLAPPED)(unaff_ESI + 2));
      if (BVar3 == 0) {
        DVar6 = GetLastError();
        if (DVar6 == 0x3e5) {
          *(undefined1 *)(unaff_ESI + 0xc) = 1;
          return;
        }
        *(undefined1 *)(unaff_ESI + 0x15) = 0;
        if ((HANDLE)*unaff_ESI == (HANDLE)0xffffffff) {
          return;
        }
        CloseHandle((HANDLE)*unaff_ESI);
        *unaff_ESI = -1;
        CloseHandle((HANDLE)unaff_ESI[1]);
        unaff_ESI[1] = 0;
        return;
      }
    }
    else {
      BVar3 = GetOverlappedResult((HANDLE)*unaff_ESI,(LPOVERLAPPED)(unaff_ESI + 2),local_8,0);
      if (BVar3 == 0) {
        DVar6 = GetLastError();
        if (DVar6 == 0x3e4) {
          return;
        }
        *(undefined1 *)(unaff_ESI + 0x15) = 0;
        if ((HANDLE)*unaff_ESI == (HANDLE)0xffffffff) {
          return;
        }
        CloseHandle((HANDLE)*unaff_ESI);
        *unaff_ESI = -1;
        CloseHandle((HANDLE)unaff_ESI[1]);
        unaff_ESI[1] = 0;
        return;
      }
      *(undefined1 *)(unaff_ESI + 0xc) = 0;
    }
    DVar6 = local_8[0];
    if (local_8[0] == 0) {
      *(undefined1 *)(unaff_ESI + 0x15) = 0;
      if ((HANDLE)*unaff_ESI == (HANDLE)0xffffffff) {
        return;
      }
      CloseHandle((HANDLE)*unaff_ESI);
      *unaff_ESI = -1;
      CloseHandle((HANDLE)unaff_ESI[1]);
      unaff_ESI[1] = 0;
      return;
    }
    uVar4 = unaff_ESI[0xe] + local_8[0];
    if ((uint)unaff_ESI[0xf] < uVar4) {
      iVar1 = uVar4 + 0x10 + (uVar4 >> 2);
      pvVar5 = realloc((void *)unaff_ESI[0xd],iVar1 + 1);
      if (pvVar5 != (void *)0x0) {
        unaff_ESI[0xd] = (int)pvVar5;
        unaff_ESI[0xf] = iVar1;
        goto LAB_10002bd5;
      }
    }
    else {
LAB_10002bd5:
      if (unaff_ESI[0xd] + unaff_ESI[0xe] != 0) {
        unaff_ESI[0xe] = unaff_ESI[0xe] + DVar6;
      }
    }
    if (unaff_ESI[0xd] != 0) {
      *(undefined1 *)(unaff_ESI[0xd] + unaff_ESI[0xe]) = 0;
    }
    uVar4 = unaff_ESI[0xe] - unaff_ESI[0x13];
    for (puVar7 = (uint *)(unaff_ESI[0xd] + unaff_ESI[0x13]);
        (3 < uVar4 && (uVar2 = *puVar7, uVar2 + 4 <= uVar4));
        puVar7 = (uint *)((int)puVar7 + uVar2 + 4)) {
      FUN_10002e20(unaff_ESI,uVar2);
      uVar4 = uVar4 + (-4 - uVar2);
    }
    uVar2 = unaff_ESI[0xe];
    uVar4 = uVar2 - uVar4;
    unaff_ESI[0x13] = uVar4;
    if ((uVar2 - uVar4 == 0) ||
       ((unaff_ESI[0xf] - uVar2 < 0x1000 && (0xfff < unaff_ESI[0xf] - (uVar2 - uVar4))))) {
      if (((int)uVar4 < 1) || (uVar2 <= uVar4)) {
        if ((unaff_ESI[0xe] != 0) && (uVar4 != 0)) {
          unaff_ESI[0xe] = 0;
        }
      }
      else {
        unaff_ESI[0xe] = uVar2 - uVar4;
        memmove((void *)unaff_ESI[0xd],(void *)(uVar4 + unaff_ESI[0xd]),uVar2 - uVar4);
      }
      unaff_ESI[0x13] = 0;
    }
    if (*unaff_ESI == -1) {
      return;
    }
  } while( true );
}



// ---- FUN_10002d40 @ 10002d40 ----

void FUN_10002d40(void)

{
  int *in_EAX;
  BOOL BVar1;
  DWORD DVar2;
  DWORD local_4;
  
  if (*in_EAX == -1) {
    return;
  }
  do {
    local_4 = 0;
    if (*(char *)((int)in_EAX + 0x31) == '\0') {
      local_4 = in_EAX[0x11] - in_EAX[0x14];
      if (local_4 == 0) {
        in_EAX[0x11] = 0;
        in_EAX[0x14] = 0;
        return;
      }
      BVar1 = WriteFile((HANDLE)*in_EAX,(LPCVOID)(in_EAX[0x10] + in_EAX[0x14]),local_4,&local_4,
                        (LPOVERLAPPED)(in_EAX + 7));
      if (BVar1 == 0) {
        DVar2 = GetLastError();
        if (DVar2 == 0x3e5) {
          *(undefined1 *)((int)in_EAX + 0x31) = 1;
          return;
        }
        goto LAB_10002dc2;
      }
    }
    else {
      BVar1 = GetOverlappedResult((HANDLE)*in_EAX,(LPOVERLAPPED)(in_EAX + 7),&local_4,0);
      if (BVar1 == 0) {
        DVar2 = GetLastError();
        if (DVar2 == 0x3e4) {
          return;
        }
LAB_10002dc2:
        FUN_10002df0();
        return;
      }
      *(undefined1 *)((int)in_EAX + 0x31) = 0;
    }
    if (local_4 == 0) goto LAB_10002dc2;
    in_EAX[0x14] = in_EAX[0x14] + local_4;
  } while( true );
}



// ---- FUN_10002df0 @ 10002df0 ----

void FUN_10002df0(void)

{
  undefined4 *unaff_ESI;
  
  *(undefined1 *)(unaff_ESI + 0x15) = 0;
  if ((HANDLE)*unaff_ESI != (HANDLE)0xffffffff) {
    CloseHandle((HANDLE)*unaff_ESI);
    *unaff_ESI = 0xffffffff;
    CloseHandle((HANDLE)unaff_ESI[1]);
    unaff_ESI[1] = 0;
  }
  return;
}



// ---- FUN_10002e20 @ 10002e20 ----

void FUN_10002e20(undefined4 *param_1,uint param_2)

{
  char *pcVar1;
  char cVar2;
  char *in_EAX;
  int iVar3;
  char *pcVar4;
  size_t sVar5;
  uint uVar6;
  char *pcVar7;
  uint local_18;
  void *local_14;
  undefined4 local_10;
  int local_c;
  undefined1 local_8 [4];
  undefined1 local_4 [4];
  
  uVar6 = 0;
  local_18 = 0;
  if ((0xb < param_2) &&
     (iVar3 = sscanf(in_EAX,"MSG#%u#%u#%u#%u%n",&local_10,local_8,local_4,&local_14,&local_c),
     iVar3 == 4)) {
    pcVar1 = in_EAX + local_c;
    for (pcVar4 = pcVar1;
        (pcVar7 = pcVar1, pcVar4 < pcVar1 + (param_2 - local_c) && (*pcVar4 != '\0'));
        pcVar4 = pcVar4 + 1) {
      if (*pcVar4 == ',') goto LAB_10002edb;
    }
    while ((pcVar4 = pcVar7, pcVar4 < pcVar1 + (param_2 - local_c) && (*pcVar4 != '\0'))) {
      if (*pcVar4 == ';') goto LAB_10002edb;
      pcVar7 = pcVar4 + 1;
    }
  }
LAB_10002ea6:
  *(undefined1 *)(param_1 + 0x15) = 0;
  if ((HANDLE)*param_1 != (HANDLE)0xffffffff) {
    CloseHandle((HANDLE)*param_1);
    *param_1 = 0xffffffff;
    CloseHandle((HANDLE)param_1[1]);
    param_1[1] = 0;
  }
  return;
LAB_10002edb:
  if (pcVar1 < pcVar4) {
    cVar2 = *pcVar1;
    pcVar7 = pcVar1;
    uVar6 = local_18;
    while (cVar2 == ':') {
      sVar5 = strspn(pcVar7 + 1,"0123456789.-");
      uVar6 = uVar6 + 1;
      pcVar7 = pcVar7 + 1 + sVar5;
      cVar2 = *pcVar7;
    }
    if (pcVar7 != pcVar4) goto LAB_10002ea6;
  }
  pcVar4 = pcVar4 + 1;
  switch(local_8[0]) {
  case 1:
    if (uVar6 != 0) {
      FUN_10003b70(local_14,pcVar4,(int)(pcVar1 + 1 + (((param_2 - local_c) + -1) - (int)pcVar4)));
      return;
    }
    break;
  default:
    if (((uint)local_14 & 1) == 0) {
      return;
    }
    FUN_10004090();
    return;
  case 0x97:
    if (4 < uVar6) {
      FUN_10003c70(pcVar1 + 1);
      return;
    }
    break;
  case 0x9a:
    FUN_100036b0(local_10,local_14,pcVar4);
    return;
  case 0x9c:
    FUN_10003aa0(pcVar4);
    return;
  }
  goto LAB_10002ea6;
}



// ---- FUN_10003080 @ 10003080 ----

uint FUN_10003080(byte *param_1,int param_2,char *param_3)

{
  int iVar1;
  int *piVar2;
  int in_EAX;
  uint uVar3;
  byte *pbVar4;
  uint uVar5;
  void *pvVar6;
  char cVar7;
  char *pcVar8;
  bool bVar9;
  char local_15;
  char *local_14;
  uint local_10;
  int local_c;
  uint local_8;
  char *local_4;
  
  local_4 = param_3 + in_EAX;
  local_15 = ',';
  if (local_4 <= param_3) {
    return 0;
  }
LAB_100030a0:
  if (local_15 == ',') {
    cVar7 = '\x01';
    uVar3 = sscanf(param_3,"%u=%u%c%n",&local_8,&local_10,&local_15,&local_c);
    uVar5 = local_10;
    if ((int)uVar3 < 3) {
switchD_100030fd_caseD_26:
      return uVar3 & 0xffffff00;
    }
    pcVar8 = param_3 + local_c;
    uVar3 = 0;
    switch(local_15) {
    case '%':
      cVar7 = '\0';
    case '/':
      cVar7 = cVar7 + -1;
    case ':':
      local_15 = pcVar8[local_10];
      param_3 = pcVar8 + local_10 + 1;
      local_14 = pcVar8;
      break;
    default:
      goto switchD_100030fd_caseD_26;
    case ',':
    case ';':
      local_14 = (char *)0x0;
      param_3 = pcVar8;
    }
    pcVar8 = local_14;
    for (pbVar4 = param_1; pbVar4 < param_1 + param_2 * 8; pbVar4 = pbVar4 + 8) {
      if ((*pbVar4 == local_8) && (*(int *)(pbVar4 + 4) != 0)) {
        switch(pbVar4[1]) {
        case 1:
          if (local_14 == (char *)0x0) {
            *(bool *)*(undefined4 *)(pbVar4 + 4) = local_10 != 0;
          }
          goto switchD_10003169_caseD_6;
        case 2:
          if (local_14 == (char *)0x0) {
            **(undefined1 **)(pbVar4 + 4) = (char)local_10;
          }
          goto switchD_10003169_caseD_6;
        case 3:
          if (local_14 == (char *)0x0) {
            **(undefined2 **)(pbVar4 + 4) = (short)local_10;
          }
          goto switchD_10003169_caseD_6;
        case 4:
          if (local_14 == (char *)0x0) {
            **(uint **)(pbVar4 + 4) = local_10;
          }
          goto switchD_10003169_caseD_6;
        case 5:
          if (local_14 != (char *)0x0) {
            piVar2 = *(int **)(pbVar4 + 4);
            if (cVar7 < '\0') {
              pbVar4 = FUN_10001290();
              FUN_100025d0(*(undefined4 *)(*piVar2 + -4),local_10,pbVar4,
                           *(undefined4 *)(*piVar2 + -4));
              FUN_10001340(piVar2);
            }
            else {
              FUN_100010d0();
              memcpy((void *)*piVar2,local_14,uVar5);
              *(uint *)(*piVar2 + -8) = uVar5;
              *(undefined1 *)(*piVar2 + uVar5) = 0;
            }
          }
        default:
          goto switchD_10003169_caseD_6;
        case 7:
          goto switchD_10003169_caseD_7;
        }
      }
    }
    goto switchD_10003169_caseD_6;
  }
  goto LAB_100032d9;
switchD_10003169_caseD_7:
  if (local_14 != (char *)0x0) {
    piVar2 = *(int **)(pbVar4 + 4);
    if (cVar7 < '\0') {
      if ((uint)piVar2[2] < piVar2[1] + (local_10 >> 2) * 3) {
        pbVar4 = (byte *)FUN_10001000();
      }
      else {
        pbVar4 = (byte *)(*piVar2 + piVar2[1]);
      }
      uVar5 = FUN_100025d0(piVar2[2],local_10,pbVar4,piVar2[2]);
      if (piVar2[1] == 0) {
        bVar9 = uVar5 <= (uint)piVar2[2];
      }
      else {
        bVar9 = uVar5 < (uint)piVar2[1];
      }
      if (bVar9) {
        piVar2[1] = uVar5;
      }
    }
    else {
      piVar2[1] = 0;
      if ((uint)piVar2[2] < local_10) {
        iVar1 = (local_10 >> 2) + 0x10 + local_10;
        pvVar6 = realloc((void *)*piVar2,iVar1 + 1);
        if (pvVar6 == (void *)0x0) goto switchD_10003169_caseD_6;
        *piVar2 = (int)pvVar6;
        piVar2[2] = iVar1;
        if (piVar2[1] + (int)pvVar6 == 0) goto switchD_10003169_caseD_6;
      }
      memcpy((void *)(*piVar2 + piVar2[1]),pcVar8,uVar5);
      piVar2[1] = uVar5;
    }
  }
switchD_10003169_caseD_6:
  if (local_4 <= param_3) {
LAB_100032d9:
    if (local_15 != ';') {
      return 0;
    }
    return 1;
  }
  goto LAB_100030a0;
}



// ---- FUN_10003350 @ 10003350 ----

void FUN_10003350(undefined4 *param_1,int *param_2)

{
  LONG *lpAddend;
  int iVar1;
  size_t _Size;
  int *piVar2;
  int *piVar3;
  undefined4 *puVar4;
  void *pvVar5;
  int iVar6;
  uint uVar7;
  int *piVar8;
  LONG LVar9;
  int iVar10;
  byte *pbVar11;
  uint local_18;
  void *local_c;
  undefined1 *puStack_8;
  undefined4 local_4;
  
  piVar2 = param_2;
  local_4 = 0xffffffff;
  puStack_8 = &LAB_10005888;
  local_c = ExceptionList;
  ExceptionList = &local_c;
  iVar6 = param_2[9];
  puVar4 = operator_new(iVar6 * 0x15 + 0x3d);
  param_2 = puVar4 + 3;
  *puVar4 = 1;
  *(undefined1 *)param_2 = 0;
  puVar4[1] = 0;
  puVar4[2] = iVar6 * 0x15 + 0x30;
  local_4 = 0;
  if ((char)piVar2[3] == '\0') {
    iVar6 = piVar2[1];
    uVar7 = iVar6 + 1;
    if ((uint)piVar2[2] < uVar7) {
      iVar10 = iVar6 + 0x11 + (uVar7 >> 2);
      pvVar5 = realloc((void *)*piVar2,iVar10 + 1);
      if (pvVar5 != (void *)0x0) {
        *piVar2 = (int)pvVar5;
        iVar6 = piVar2[1];
        piVar2[2] = iVar10;
        if (*piVar2 + iVar6 != 0) goto LAB_100033f1;
      }
    }
    else {
LAB_100033f1:
      *(undefined1 *)(*piVar2 + iVar6) = 0x3b;
      piVar2[1] = uVar7;
    }
    *(undefined1 *)(piVar2 + 3) = 1;
  }
  FUN_10002580(piVar2[6],(int *)&param_2,(uchar *)"MSG#%u#%u#%u#%u");
  local_18 = 0;
  if (piVar2[9] != 0) {
    pbVar11 = (byte *)(piVar2 + 10);
    do {
      piVar3 = param_2;
      lpAddend = param_2 + -3;
      if ((param_2[-3] < 2) && (param_2[-2] + 1 <= param_2[-1])) {
        *(undefined1 *)(param_2[-2] + (int)param_2) = 0x3a;
        param_2[-2] = param_2[-2] + 1;
        *(undefined1 *)((int)param_2 + param_2[-2]) = 0;
      }
      else {
        _Size = param_2[-2];
        iVar6 = _Size + 1;
        if (iVar6 != 0) {
          iVar10 = ((int)(iVar6 + (iVar6 >> 0x1f & 3U)) >> 2) + iVar6;
          puVar4 = operator_new(iVar10 + 0xd);
          *puVar4 = 1;
          *(undefined1 *)((int)puVar4 + _Size + 0xd) = 0;
          puVar4[2] = iVar10;
          piVar8 = puVar4 + 3;
          puVar4[1] = iVar6;
          param_2 = piVar8;
          memcpy(piVar8,piVar3,_Size);
          *(undefined1 *)(_Size + (int)piVar8) = 0x3a;
        }
        if ((lpAddend != (LONG *)&DAT_100085ac) &&
           (LVar9 = InterlockedDecrement(lpAddend), LVar9 < 1)) {
          operator_delete(lpAddend);
        }
      }
      FUN_100025a0(&DAT_10006dc8 + (uint)*pbVar11 * 5);
      local_18 = local_18 + 1;
      pbVar11 = pbVar11 + 0x10;
    } while (local_18 < (uint)piVar2[9]);
  }
  piVar3 = param_2;
  iVar6 = piVar2[1];
  iVar10 = param_2[-2];
  param_2 = (int *)(iVar10 + iVar6 + 4);
  uVar7 = param_1[0x11] + (int)param_2;
  if ((uint)param_1[0x12] < uVar7) {
    iVar1 = uVar7 + 0x10 + (uVar7 >> 2);
    pvVar5 = realloc((void *)param_1[0x10],iVar1 + 1);
    if (pvVar5 != (void *)0x0) {
      param_1[0x10] = pvVar5;
      param_1[0x12] = iVar1;
      goto LAB_10003573;
    }
  }
  else {
LAB_10003573:
    piVar8 = (int *)(param_1[0x10] + param_1[0x11]);
    if (piVar8 != (int *)0x0) {
      param_1[0x11] = param_1[0x11] + (int)param_2;
      *piVar8 = iVar10 + iVar6;
      memcpy(piVar8 + 1,piVar3,piVar3[-2]);
      memcpy((void *)((int)(piVar8 + 1) + piVar3[-2]),(void *)*piVar2,piVar2[1]);
      FUN_10002d40();
      goto LAB_100035bd;
    }
  }
  *(undefined1 *)(param_1 + 0x15) = 0;
  if ((HANDLE)*param_1 != (HANDLE)0xffffffff) {
    CloseHandle((HANDLE)*param_1);
    *param_1 = 0xffffffff;
    CloseHandle((HANDLE)param_1[1]);
    param_1[1] = 0;
  }
LAB_100035bd:
  local_4 = 0xffffffff;
  if (piVar3 != (int *)&DAT_100085b8) {
    LVar9 = InterlockedDecrement(piVar3 + -3);
    if (LVar9 < 1) {
      operator_delete(piVar3 + -3);
    }
  }
  ExceptionList = local_c;
  return;
}



// ---- FUN_10003630 @ 10003630 ----

uint __fastcall FUN_10003630(int param_1,int param_2)

{
  uint uVar1;
  int *in_EAX;
  int *piVar2;
  int iVar3;
  uint uVar4;
  int *piVar5;
  void *pvVar6;
  
  piVar2 = *(int **)(param_2 + 0x5c);
  piVar5 = piVar2 + *(int *)(param_2 + 0x60) * 4;
  while( true ) {
    if (piVar5 <= piVar2) {
      return (uint)piVar2 & 0xffffff00;
    }
    if (*piVar2 == param_1) break;
    piVar2 = piVar2 + 4;
  }
  *in_EAX = *piVar2;
  in_EAX[1] = piVar2[1];
  in_EAX[2] = piVar2[2];
  in_EAX[3] = piVar2[3];
  uVar1 = *(uint *)(param_2 + 0x60);
  iVar3 = (int)piVar2 - *(int *)(param_2 + 0x5c);
  uVar4 = iVar3 >> 4;
  if (uVar1 <= uVar4 + 1) {
    if (uVar4 < uVar1) {
      *(uint *)(param_2 + 0x60) = uVar4;
    }
    return CONCAT31((int3)(iVar3 >> 0xc),1);
  }
  *(uint *)(param_2 + 0x60) = uVar1 - 1;
  pvVar6 = (void *)(uVar4 * 0x10 + *(int *)(param_2 + 0x5c));
  pvVar6 = memmove(pvVar6,(void *)((int)pvVar6 + 0x10),((uVar1 - 1) - uVar4) * 0x10);
  return CONCAT31((int3)((uint)pvVar6 >> 8),1);
}



// ---- FUN_100036b0 @ 100036b0 ----

void FUN_100036b0(undefined4 param_1,undefined4 param_2,char *param_3)

{
  undefined4 *lParam;
  DWORD DVar1;
  LONG LVar2;
  undefined1 *local_7c;
  undefined1 *local_78;
  undefined1 *local_74;
  undefined1 *local_70;
  undefined1 *local_6c;
  undefined1 *local_68;
  undefined1 *local_64;
  undefined1 *local_60;
  undefined4 local_5c;
  byte local_58 [4];
  undefined4 *local_54;
  undefined1 local_50;
  undefined1 local_4f;
  undefined **local_4c;
  undefined1 local_48;
  undefined1 local_47;
  undefined **local_44;
  undefined1 local_40;
  undefined1 local_3f;
  undefined **local_3c;
  undefined1 local_38;
  undefined1 local_37;
  undefined **local_34;
  undefined1 local_30;
  undefined1 local_2f;
  undefined **local_2c;
  undefined1 local_28;
  undefined1 local_27;
  undefined **local_24;
  undefined1 local_20;
  undefined1 local_1f;
  undefined **local_1c;
  undefined1 local_18;
  undefined1 local_17;
  undefined **local_14;
  void *local_c;
  undefined1 *puStack_8;
  int local_4;
  
  puStack_8 = &LAB_10005850;
  local_c = ExceptionList;
  ExceptionList = &local_c;
  local_5c = 0;
  local_60 = PTR_DAT_100085c0;
  local_64 = PTR_DAT_100085c0;
  local_68 = PTR_DAT_100085c0;
  local_6c = PTR_DAT_100085c0;
  local_70 = PTR_DAT_100085c0;
  local_74 = PTR_DAT_100085c0;
  local_78 = PTR_DAT_100085c0;
  local_7c = PTR_DAT_100085c0;
  local_4 = 7;
  local_54 = &local_5c;
  local_3c = &local_68;
  local_4c = &local_60;
  local_24 = &local_74;
  local_44 = &local_64;
  local_34 = &local_6c;
  local_2c = &local_70;
  local_1c = &local_78;
  local_14 = &local_7c;
  local_58[0] = 0x10;
  local_58[1] = 4;
  local_50 = 6;
  local_4f = 5;
  local_48 = 0x36;
  local_47 = 5;
  local_40 = 0x39;
  local_3f = 5;
  local_38 = 0x33;
  local_37 = 5;
  local_30 = 0x3b;
  local_2f = 5;
  local_28 = 0x3a;
  local_27 = 5;
  local_20 = 0xe;
  local_1f = 5;
  local_18 = 0x7a;
  local_17 = 5;
  FUN_10003080(local_58,9,param_3);
  lParam = operator_new(0x30);
  if (lParam != (undefined4 *)0x0) {
    lParam[3] = PTR_DAT_100085c0;
    lParam[4] = PTR_DAT_100085c0;
    lParam[5] = PTR_DAT_100085c0;
    lParam[6] = PTR_DAT_100085c0;
    lParam[7] = PTR_DAT_100085c0;
    lParam[8] = PTR_DAT_100085c0;
    lParam[9] = PTR_DAT_100085c0;
    lParam[10] = PTR_DAT_100085c0;
    *lParam = local_5c;
    lParam[1] = param_1;
    lParam[2] = param_2;
    FUN_10001200(lParam + 3);
    FUN_10001200(lParam + 4);
    FUN_10001200(lParam + 5);
    FUN_10001200(lParam + 6);
    FUN_10001200(lParam + 7);
    FUN_10001200(lParam + 8);
    FUN_10001200(lParam + 9);
    DVar1 = GetCurrentThreadId();
    lParam[0xb] = DVar1;
    FUN_10001200(lParam + 10);
    PostMessageA(DAT_1000893c,0x2b4e,0,(LPARAM)lParam);
  }
  local_4._0_1_ = 6;
  if (local_7c != &DAT_100085b8) {
    LVar2 = InterlockedDecrement((LONG *)(local_7c + -0xc));
    if (LVar2 < 1) {
      operator_delete(local_7c + -0xc);
    }
  }
  local_4._0_1_ = 5;
  if (local_78 != &DAT_100085b8) {
    LVar2 = InterlockedDecrement((LONG *)(local_78 + -0xc));
    if (LVar2 < 1) {
      operator_delete(local_78 + -0xc);
    }
  }
  local_4._0_1_ = 4;
  if (local_74 != &DAT_100085b8) {
    LVar2 = InterlockedDecrement((LONG *)(local_74 + -0xc));
    if (LVar2 < 1) {
      operator_delete(local_74 + -0xc);
    }
  }
  local_4._0_1_ = 3;
  if (local_70 != &DAT_100085b8) {
    LVar2 = InterlockedDecrement((LONG *)(local_70 + -0xc));
    if (LVar2 < 1) {
      operator_delete(local_70 + -0xc);
    }
  }
  local_4._0_1_ = 2;
  if (local_6c != &DAT_100085b8) {
    LVar2 = InterlockedDecrement((LONG *)(local_6c + -0xc));
    if (LVar2 < 1) {
      operator_delete(local_6c + -0xc);
    }
  }
  local_4._0_1_ = 1;
  if (local_68 != &DAT_100085b8) {
    LVar2 = InterlockedDecrement((LONG *)(local_68 + -0xc));
    if (LVar2 < 1) {
      operator_delete(local_68 + -0xc);
    }
  }
  local_4 = (uint)local_4._1_3_ << 8;
  if (local_64 != &DAT_100085b8) {
    LVar2 = InterlockedDecrement((LONG *)(local_64 + -0xc));
    if (LVar2 < 1) {
      operator_delete(local_64 + -0xc);
    }
  }
  local_4 = 0xffffffff;
  if (local_60 != &DAT_100085b8) {
    LVar2 = InterlockedDecrement((LONG *)(local_60 + -0xc));
    if (LVar2 < 1) {
      operator_delete(local_60 + -0xc);
    }
  }
  ExceptionList = local_c;
  return;
}



// ---- FUN_10003aa0 @ 10003aa0 ----

void FUN_10003aa0(char *param_1)

{
  void *pvVar1;
  byte local_20 [4];
  void **local_1c;
  void *local_18;
  int local_14;
  undefined4 local_10;
  void *local_c;
  undefined1 *puStack_8;
  undefined4 local_4;
  
  local_4 = 0xffffffff;
  puStack_8 = &LAB_100057e8;
  local_c = ExceptionList;
  ExceptionList = &local_c;
  local_18 = (void *)0x0;
  local_10 = 0;
  local_14 = 0;
  pvVar1 = realloc((void *)0x0,0x2a);
  if (pvVar1 != (void *)0x0) {
    local_10 = 0x29;
    local_18 = pvVar1;
  }
  local_4 = 0;
  local_1c = &local_18;
  local_20[0] = 0x91;
  local_20[1] = 7;
  FUN_10003080(local_20,1,param_1);
  if (local_14 == 0x14) {
    SendMessageA(DAT_1000893c,0x2b4f,0,(LPARAM)local_18);
  }
  if (local_18 != (void *)0x0) {
    free(local_18);
  }
  ExceptionList = local_c;
  return;
}



// ---- FUN_10003b70 @ 10003b70 ----

void __thiscall FUN_10003b70(void *this,char *param_1,int param_2)

{
  int iVar1;
  char *in_EAX;
  int iVar2;
  undefined4 uVar3;
  undefined4 *unaff_ESI;
  CHAR local_14 [4];
  int local_10;
  
  iVar1 = param_2;
  iVar2 = sscanf(in_EAX,"%u:",&param_2);
  if (iVar2 == 1) {
    uVar3 = FUN_10003630(param_2,(int)unaff_ESI);
    if ((char)uVar3 != '\0') {
      if (local_10 == 0x96) {
        if (((uint)this & 2) == 0) {
          *(undefined1 *)(unaff_ESI + 0x15) = 0;
          if ((HANDLE)*unaff_ESI != (HANDLE)0xffffffff) {
            CloseHandle((HANDLE)*unaff_ESI);
            *unaff_ESI = 0xffffffff;
            CloseHandle((HANDLE)unaff_ESI[1]);
            unaff_ESI[1] = 0;
          }
          DAT_10008934 = 1;
        }
      }
      else {
        if (local_10 == 0x98) {
          FUN_10003ce0(unaff_ESI,(uint)this,in_EAX,param_1,iVar1,local_14);
          return;
        }
        if (local_10 == 0x99) {
          FUN_10003ec0(unaff_ESI,(byte)this,in_EAX,param_1,iVar1,local_14);
          return;
        }
      }
    }
  }
  else {
    *(undefined1 *)(unaff_ESI + 0x15) = 0;
    if ((HANDLE)*unaff_ESI != (HANDLE)0xffffffff) {
      CloseHandle((HANDLE)*unaff_ESI);
      *unaff_ESI = 0xffffffff;
      CloseHandle((HANDLE)unaff_ESI[1]);
      unaff_ESI[1] = 0;
      return;
    }
  }
  return;
}



// ---- FUN_10003c70 @ 10003c70 ----

void FUN_10003c70(char *param_1)

{
  int iVar1;
  undefined4 *unaff_ESI;
  undefined1 local_8 [4];
  undefined1 local_4 [4];
  
  iVar1 = sscanf(param_1,"%hu:%hu:%u:%u:%u",unaff_ESI + 0x42,&param_1,unaff_ESI + 0x43,local_4,
                 local_8);
  if (iVar1 < 5) {
    *(undefined1 *)(unaff_ESI + 0x15) = 0;
    if ((HANDLE)*unaff_ESI != (HANDLE)0xffffffff) {
      CloseHandle((HANDLE)*unaff_ESI);
      *unaff_ESI = 0xffffffff;
      CloseHandle((HANDLE)unaff_ESI[1]);
      unaff_ESI[1] = 0;
      return;
    }
  }
  else {
    *(undefined1 *)(unaff_ESI + 0x15) = 1;
  }
  return;
}



// ---- FUN_10003ce0 @ 10003ce0 ----

/* WARNING: Removing unreachable block (ram,0x10003e02) */

void __thiscall
FUN_10003ce0(void *this,uint param_1,char *param_2,char *param_3,undefined4 param_4,LPCSTR param_5)

{
  undefined4 *puVar1;
  HANDLE hEvent;
  LPCSTR pCVar2;
  int iVar3;
  byte local_18;
  undefined1 local_17;
  LPCSTR *local_14;
  void *local_c;
  undefined1 *puStack_8;
  undefined4 local_4;
  
  pCVar2 = param_5;
  puStack_8 = &LAB_100057c0;
  local_c = ExceptionList;
  param_5 = PTR_DAT_100085c0;
  local_4 = 0;
  puVar1 = *(undefined4 **)(pCVar2 + 0xc);
  local_14 = &param_5;
  local_18 = 0x33;
  local_17 = 5;
  if (puVar1 == (undefined4 *)0x0) {
    local_4 = 0xffffffff;
    if (PTR_DAT_100085c0 == &DAT_100085b8) {
      return;
    }
    ExceptionList = &local_c;
    iVar3 = InterlockedDecrement((LONG *)(PTR_DAT_100085c0 + -0xc));
    goto joined_r0x10003e94;
  }
  ExceptionList = &local_c;
  iVar3 = sscanf(param_2,"%*u:%u");
  if ((iVar3 < 1) && ((param_1 & 2) != 0)) {
    puVar1[5] = 0xf;
LAB_10003d9b:
    *(undefined1 *)((int)this + 0x54) = 0;
    if (*(HANDLE *)this != (HANDLE)0xffffffff) {
      CloseHandle(*(HANDLE *)this);
      *(undefined4 *)this = 0xffffffff;
      CloseHandle(*(HANDLE *)((int)this + 4));
      *(undefined4 *)((int)this + 4) = 0;
    }
  }
  else {
    if (*(int *)(pCVar2 + 8) == 0) {
      puVar1[5] = 0x10;
      goto LAB_10003d9b;
    }
    if ((param_1 & 2) == 0) {
      if ((param_1 & 8) == 0) {
        puVar1[5] = 4;
        if ((param_1 & 0x10) != 0) {
          puVar1[5] = 0x14;
        }
      }
      else {
        puVar1[5] = 3;
      }
    }
    else {
      FUN_10003080(&local_18,1,param_3);
      puVar1[5] = 10;
    }
    puVar1[6] = 1;
    hEvent = (HANDLE)*puVar1;
    pCVar2[0xc] = '\0';
    pCVar2[0xd] = '\0';
    pCVar2[0xe] = '\0';
    pCVar2[0xf] = '\0';
    if (hEvent != (HANDLE)0x0) {
      SetEvent(hEvent);
    }
  }
  local_4 = 0xffffffff;
  if (param_5 == &DAT_100085b8) {
    ExceptionList = local_c;
    return;
  }
  iVar3 = InterlockedDecrement((LONG *)(param_5 + -0xc));
joined_r0x10003e94:
  if (iVar3 < 1) {
    operator_delete(param_5 + -0xc);
  }
  ExceptionList = local_c;
  return;
}



// ---- FUN_10003ec0 @ 10003ec0 ----

/* WARNING: Removing unreachable block (ram,0x1000401f) */

void __thiscall
FUN_10003ec0(void *this,byte param_1,char *param_2,char *param_3,undefined4 param_4,
            undefined1 *param_5)

{
  undefined4 *puVar1;
  HANDLE hEvent;
  undefined1 *puVar2;
  int iVar3;
  code *pcVar4;
  byte local_18;
  undefined1 local_17;
  undefined1 **local_14;
  void *local_c;
  undefined1 *puStack_8;
  undefined4 local_4;
  
  puVar2 = param_5;
  puStack_8 = &LAB_10005788;
  local_c = ExceptionList;
  ExceptionList = &local_c;
  puVar1 = *(undefined4 **)(param_5 + 0xc);
  param_5 = PTR_DAT_100085c0;
  local_4 = 0;
  local_18 = 5;
  local_17 = 5;
  local_14 = &param_5;
  iVar3 = sscanf(param_2,"%*u:%hu:%u");
  if ((iVar3 < 2) && ((param_1 & 2) != 0)) {
    if (puVar1 != (undefined4 *)0x0) {
      puVar1[5] = 0xb;
    }
    *(undefined1 *)((int)this + 0x54) = 0;
    pcVar4 = CloseHandle_exref;
    if (*(HANDLE *)this != (HANDLE)0xffffffff) {
      CloseHandle(*(HANDLE *)this);
LAB_10003f64:
      *(undefined4 *)this = 0xffffffff;
      (*pcVar4)();
      *(undefined4 *)((int)this + 4) = 0;
    }
  }
  else {
    if (*(int *)(puVar2 + 8) != 0) {
      FUN_10003080(&local_18,1,param_3);
      if (puVar1 == (undefined4 *)0x0) {
        *(undefined4 *)(puVar2 + 0xc) = 0;
      }
      else {
        if ((param_1 & 2) == 0) {
          puVar1[5] = 2 - (uint)((param_1 & 8) != 0);
        }
        puVar1[5] = 0x11;
        puVar1[6] = 1;
        hEvent = (HANDLE)*puVar1;
        *(undefined4 *)(puVar2 + 0xc) = 0;
        if (hEvent != (HANDLE)0x0) {
          SetEvent(hEvent);
        }
      }
      local_4 = 0xffffffff;
      if (param_5 == &DAT_100085b8) {
        ExceptionList = local_c;
        return;
      }
      iVar3 = InterlockedDecrement((LONG *)(param_5 + -0xc));
      goto joined_r0x10004064;
    }
    if (puVar1 != (undefined4 *)0x0) {
      puVar1[5] = 0xc;
    }
    *(undefined1 *)((int)this + 0x54) = 0;
    pcVar4 = CloseHandle_exref;
    if (*(HANDLE *)this != (HANDLE)0xffffffff) {
      CloseHandle(*(HANDLE *)this);
      goto LAB_10003f64;
    }
  }
  local_4 = 0xffffffff;
  if (param_5 == &DAT_100085b8) {
    ExceptionList = local_c;
    return;
  }
  iVar3 = InterlockedDecrement((LONG *)(param_5 + -0xc));
joined_r0x10004064:
  if (iVar3 < 1) {
    operator_delete(param_5 + -0xc);
  }
  ExceptionList = local_c;
  return;
}



// ---- FUN_10004090 @ 10004090 ----

void FUN_10004090(void)

{
  undefined4 *unaff_ESI;
  int unaff_EDI;
  void *local_d8;
  undefined4 local_d4;
  undefined4 local_d0;
  undefined1 local_cc;
  int local_c8;
  undefined4 local_c4;
  undefined4 local_c0;
  undefined4 local_bc;
  undefined1 *local_b8;
  int local_b4;
  undefined1 local_b0 [8];
  void *local_c;
  undefined1 *puStack_8;
  undefined4 local_4;
  
  local_4 = 0xffffffff;
  puStack_8 = &LAB_1000595b;
  local_c = ExceptionList;
  ExceptionList = &local_c;
  if (unaff_EDI != 0) {
    local_c8 = unaff_ESI[0x16];
    local_b8 = local_b0;
    local_c4 = 1;
    local_b4 = 1;
    unaff_ESI[0x16] = local_c8 + 1;
    local_d8 = (void *)0x0;
    local_d0 = 0;
    local_d4 = 0;
    local_cc = 0;
    local_c0 = 2;
    local_bc = 0;
    FUN_10002500();
    memset(local_b8,0,local_b4 << 4);
    local_4 = 0;
    local_b0[0] = 1;
    FUN_10003350(unaff_ESI,(int *)&local_d8);
    if (local_d8 != (void *)0x0) {
      free(local_d8);
    }
  }
  ExceptionList = local_c;
  return;
}



// ---- FUN_10004170 @ 10004170 ----

void __fastcall FUN_10004170(undefined4 *param_1)

{
  int iVar1;
  LSTATUS LVar2;
  int iVar3;
  char *pcVar4;
  undefined4 uVar5;
  int *piVar6;
  undefined4 extraout_EDX;
  undefined4 extraout_EDX_00;
  undefined4 extraout_EDX_01;
  undefined4 uVar7;
  uint uVar8;
  uint uVar9;
  undefined1 auStack_130 [6];
  char local_12a;
  char local_129;
  char *local_128;
  HKEY local_124;
  uint local_120;
  DWORD local_11c;
  void *local_118;
  undefined4 local_114;
  undefined4 local_110;
  undefined1 local_10c;
  int local_108;
  int local_104;
  undefined4 local_100;
  undefined4 local_fc;
  undefined1 *local_f8;
  int local_f4;
  undefined1 local_f0 [8];
  int local_e8;
  undefined4 local_e4;
  undefined1 local_e0;
  uint local_d8;
  undefined4 local_d4;
  undefined1 local_d0;
  DWORD local_c8;
  undefined4 local_c4;
  BYTE local_50;
  char local_4f [63];
  uint local_10;
  void *local_c;
  undefined1 *puStack_8;
  undefined4 local_4;
  
  uVar8 = DAT_10008928;
  local_4 = 0xffffffff;
  puStack_8 = &LAB_1000591b;
  local_c = ExceptionList;
  local_10 = DAT_1000828c ^ (uint)auStack_130;
  ExceptionList = &local_c;
  local_120 = DAT_10008928;
  uVar9 = uVar8;
  if ((DAT_10008928 == 0) &&
     (LVar2 = RegOpenKeyExA((HKEY)0x80000001,"Software\\DownloadManager",0,0xf003f,&local_124),
     LVar2 == 0)) {
    local_11c = 0x40;
    LVar2 = RegQueryValueExA(local_124,"idmvers",(LPDWORD)0x0,(LPDWORD)0x0,&local_50,&local_11c);
    if ((LVar2 == 0) &&
       ((local_50 == 'v' && (local_128 = strchr(local_4f,0x2e), local_128 != (char *)0x0)))) {
      *local_128 = '\0';
      iVar3 = atoi(local_4f);
      local_129 = (char)iVar3;
      local_128 = local_128 + 1;
      pcVar4 = strchr(local_128,0x62);
      if (pcVar4 != (char *)0x0) {
        *pcVar4 = '\0';
        iVar3 = atoi(local_128);
        local_12a = (char)iVar3;
        iVar3 = atoi(pcVar4 + 1);
        if (((local_129 != '\0') && (local_12a != '\0')) && ((char)iVar3 != '\0')) {
          uVar8 = local_120;
          uVar9 = CONCAT22(CONCAT11(local_129,local_12a),(short)iVar3 << 8) | 1;
        }
      }
    }
    RegCloseKey(local_124);
  }
  local_108 = param_1[0x16];
  local_f8 = local_f0;
  local_100 = 1;
  local_fc = 1;
  param_1[0x16] = local_108 + 1;
  local_118 = (void *)0x0;
  local_110 = 0;
  local_114 = 0;
  local_10c = 0;
  local_104 = 0x96;
  local_f4 = 3;
  FUN_10002500();
  memset(local_f8,0,local_f4 << 4);
  local_4 = 0;
  local_e8 = (-(uint)(uVar8 != 0) & 0x15) + 0x5c;
  local_f0[0] = 0;
  local_e4 = 0;
  local_e0 = 1;
  local_d4 = 0;
  local_d8 = uVar9;
  local_c8 = GetCurrentProcessId();
  local_d0 = 1;
  local_c4 = 0;
  FUN_10003350(param_1,(int *)&local_118);
  iVar1 = local_104;
  iVar3 = local_108;
  uVar7 = extraout_EDX;
  if (((uint)param_1[0x18] < (uint)param_1[0x19]) ||
     (uVar5 = FUN_10004a00(param_1[0x19] + 10), uVar7 = extraout_EDX_00, (char)uVar5 != '\0')) {
    piVar6 = (int *)(param_1[0x18] * 0x10 + param_1[0x17]);
    if (piVar6 != (int *)0x0) {
      *piVar6 = iVar3;
      piVar6[1] = iVar1;
      piVar6[2] = 0;
      piVar6[3] = 0;
    }
    param_1[0x18] = param_1[0x18] + 1;
  }
  if (local_118 != (void *)0x0) {
    free(local_118);
    uVar7 = extraout_EDX_01;
  }
  ExceptionList = local_c;
  FUN_10004ab0(local_10 ^ (uint)auStack_130,uVar7);
  return;
}



// ---- FUN_10004410 @ 10004410 ----

void FUN_10004410(undefined4 param_1,undefined4 *param_2,undefined4 *param_3,undefined4 *param_4)

{
  undefined4 *puVar1;
  int unaff_ESI;
  void *local_d8;
  undefined4 local_d4;
  undefined4 local_d0;
  undefined1 local_cc;
  int local_c8;
  undefined4 local_c4;
  undefined4 local_c0;
  undefined4 local_bc;
  undefined1 *local_b8;
  int local_b4;
  undefined1 local_b0 [8];
  undefined4 local_a8;
  undefined4 local_a4;
  undefined1 local_a0;
  void *local_c;
  undefined1 *puStack_8;
  undefined4 local_4;
  
  puVar1 = DAT_1000892c;
  local_4 = 0xffffffff;
  puStack_8 = &LAB_1000595b;
  local_c = ExceptionList;
  ExceptionList = &local_c;
  local_bc = 0x32;
  if ((unaff_ESI != 200) && (unaff_ESI != 0xce)) {
    local_bc = 4;
  }
  local_c8 = DAT_1000892c[0x16];
  local_b8 = local_b0;
  local_c0 = 2;
  local_b4 = 2;
  DAT_1000892c[0x16] = local_c8 + 1;
  local_d8 = (void *)0x0;
  local_d0 = 0;
  local_d4 = 0;
  local_cc = 0;
  local_c4 = 1;
  FUN_10002500();
  memset(local_b8,0,local_b4 << 4);
  local_4 = 0;
  local_b0[0] = 1;
  local_a8 = param_1;
  local_a4 = 0;
  local_a0 = 1;
  if ((unaff_ESI == 200) || (unaff_ESI == 0xce)) {
    FUN_10002890(&local_d8,(void *)*param_2,*(size_t *)((int)*param_2 + -8));
    FUN_10002890(&local_d8,(void *)*param_3,*(size_t *)((int)*param_3 + -8));
    FUN_10002890(&local_d8,(void *)*param_4,*(size_t *)((int)*param_4 + -8));
  }
  FUN_10003350(puVar1,(int *)&local_d8);
  if (local_d8 != (void *)0x0) {
    free(local_d8);
  }
  ExceptionList = local_c;
  return;
}



// ---- FUN_10004580 @ 10004580 ----

undefined4 __thiscall FUN_10004580(void *this,undefined4 param_1,int param_2,int param_3)

{
  undefined4 *puVar1;
  int iVar2;
  int iVar3;
  undefined4 uVar4;
  int *piVar5;
  undefined4 extraout_EAX;
  void **ppvVar6;
  void *local_dc;
  undefined4 local_d8;
  undefined4 local_d4;
  undefined1 local_d0;
  int local_cc;
  int local_c8;
  undefined4 local_c4;
  undefined4 local_c0;
  undefined1 *local_bc;
  int local_b8;
  undefined1 local_b4 [8];
  undefined4 local_ac;
  undefined4 local_a8;
  undefined1 local_a4;
  undefined4 local_9c;
  undefined4 local_98;
  void *local_c;
  undefined1 *puStack_8;
  undefined4 local_4;
  
  puVar1 = DAT_1000892c;
  local_4 = 0xffffffff;
  puStack_8 = &LAB_100058eb;
  local_c = ExceptionList;
  ppvVar6 = &local_c;
  if ((*(char *)(DAT_1000892c + 0x15) != '\0') &&
     (ppvVar6 = *(void ***)this, ppvVar6[-2] != (void *)0x0)) {
    local_cc = DAT_1000892c[0x16];
    local_bc = local_b4;
    local_c4 = 1;
    local_c0 = 1;
    ExceptionList = &local_c;
    DAT_1000892c[0x16] = local_cc + 1;
    local_dc = (void *)0x0;
    local_d4 = 0;
    local_d8 = 0;
    local_d0 = 0;
    local_c8 = 0x98;
    local_b8 = 2;
    FUN_10002500();
    memset(local_bc,0,local_b8 << 4);
    local_4 = 0;
    local_b4[0] = 0;
    local_ac = 0;
    local_a8 = 0;
    local_a4 = 1;
    local_9c = param_1;
    local_98 = 0;
    FUN_10002890(&local_dc,*(void **)this,*(size_t *)((int)*(void **)this + -8));
    FUN_10003350(puVar1,(int *)&local_dc);
    iVar3 = local_c8;
    iVar2 = local_cc;
    if (((uint)puVar1[0x18] < (uint)puVar1[0x19]) ||
       (uVar4 = FUN_10004a00(puVar1[0x19] + 10), (char)uVar4 != '\0')) {
      piVar5 = (int *)(puVar1[0x18] * 0x10 + puVar1[0x17]);
      if (piVar5 != (int *)0x0) {
        *piVar5 = iVar2;
        piVar5[1] = iVar3;
        piVar5[2] = param_2;
        piVar5[3] = param_3;
      }
      puVar1[0x18] = puVar1[0x18] + 1;
    }
    uVar4 = 0;
    if (local_dc != (void *)0x0) {
      free(local_dc);
      uVar4 = extraout_EAX;
    }
    ExceptionList = local_c;
    return CONCAT31((int3)((uint)uVar4 >> 8),1);
  }
  return (uint)ppvVar6 & 0xffffff00;
}



// ---- FUN_100046f0 @ 100046f0 ----

undefined4 FUN_100046f0(int param_1,int param_2)

{
  undefined4 *puVar1;
  int iVar2;
  int iVar3;
  undefined4 uVar4;
  int *piVar5;
  undefined4 extraout_EAX;
  undefined1 auStackY_100 [4];
  undefined4 uStackY_fc;
  void *local_d8;
  undefined4 local_d4;
  undefined4 local_d0;
  undefined1 local_cc;
  int local_c8;
  int local_c4;
  undefined4 local_c0;
  undefined4 local_bc;
  undefined1 *local_b8;
  int local_b4;
  undefined1 local_b0 [8];
  undefined4 local_a8;
  undefined4 local_a4;
  void *local_c;
  undefined1 *puStack_8;
  undefined4 local_4;
  
  puVar1 = DAT_1000892c;
  local_4 = 0xffffffff;
  puStack_8 = &LAB_100058bb;
  local_c = ExceptionList;
  if (*(char *)(DAT_1000892c + 0x15) != '\0') {
    local_c8 = DAT_1000892c[0x16];
    local_b8 = local_b0;
    local_c0 = 1;
    local_bc = 1;
    local_b4 = 1;
    ExceptionList = &local_c;
    DAT_1000892c[0x16] = local_c8 + 1;
    local_d8 = (void *)0x0;
    local_d0 = 0;
    local_d4 = 0;
    local_cc = 0;
    local_c4 = 0x99;
    FUN_10002500();
    uStackY_fc = 0x10004789;
    memset(local_b8,0,local_b4 << 4);
    local_4 = 0;
    local_b0[0] = 0;
    local_a8 = 0;
    local_a4 = 0;
    FUN_10003350(puVar1,(int *)&local_d8);
    iVar3 = local_c4;
    iVar2 = local_c8;
    if (((uint)puVar1[0x18] < (uint)puVar1[0x19]) ||
       (uVar4 = FUN_10004a00(puVar1[0x19] + 10), (char)uVar4 != '\0')) {
      piVar5 = (int *)(puVar1[0x18] * 0x10 + puVar1[0x17]);
      if (piVar5 != (int *)0x0) {
        *piVar5 = iVar2;
        piVar5[1] = iVar3;
        piVar5[2] = param_1;
        piVar5[3] = param_2;
      }
      puVar1[0x18] = puVar1[0x18] + 1;
    }
    uVar4 = 0;
    if (local_d8 != (void *)0x0) {
      free(local_d8);
      uVar4 = extraout_EAX;
    }
    ExceptionList = local_c;
    return CONCAT31((int3)((uint)uVar4 >> 8),1);
  }
  return auStackY_100;
}



// ---- FUN_10004820 @ 10004820 ----

void FUN_10004820(void)

{
  undefined4 *puVar1;
  void *local_d8;
  undefined4 local_d4;
  undefined4 local_d0;
  undefined1 local_cc;
  int local_c8;
  undefined4 local_c4;
  undefined4 local_c0;
  undefined4 local_bc;
  undefined1 *local_b8;
  int local_b4;
  undefined1 local_b0 [164];
  void *local_c;
  undefined1 *puStack_8;
  undefined4 local_4;
  
  puVar1 = DAT_1000892c;
  local_4 = 0xffffffff;
  puStack_8 = &LAB_1000595b;
  local_c = ExceptionList;
  ExceptionList = &local_c;
  if (*(char *)(DAT_1000892c + 0x15) != '\0') {
    local_c8 = DAT_1000892c[0x16];
    local_b8 = local_b0;
    DAT_1000892c[0x16] = local_c8 + 1;
    local_d8 = (void *)0x0;
    local_d0 = 0;
    local_d4 = 0;
    local_cc = 0;
    local_c4 = 0x9d;
    local_c0 = 1;
    local_bc = 0;
    local_b4 = 0;
    FUN_10002500();
    memset(local_b8,0,local_b4 << 4);
    local_4 = 0;
    FUN_10003350(puVar1,(int *)&local_d8);
    if (local_d8 != (void *)0x0) {
      free(local_d8);
    }
  }
  ExceptionList = local_c;
  return;
}



// ---- FUN_100048f0 @ 100048f0 ----

void FUN_100048f0(void)

{
  undefined4 *puVar1;
  void *local_dc;
  undefined4 local_d8;
  undefined4 local_d4;
  undefined1 local_d0;
  int local_cc;
  undefined4 local_c8;
  undefined4 local_c4;
  undefined4 local_c0;
  undefined1 *local_bc;
  int local_b8;
  undefined1 local_b4 [8];
  undefined4 local_ac;
  undefined4 local_a8;
  void *local_c;
  undefined1 *puStack_8;
  undefined4 local_4;
  
  puVar1 = DAT_1000892c;
  local_4 = 0xffffffff;
  puStack_8 = &LAB_100058eb;
  local_c = ExceptionList;
  ExceptionList = &local_c;
  if (*(char *)(DAT_1000892c + 0x15) != '\0') {
    local_cc = DAT_1000892c[0x16];
    local_bc = local_b4;
    DAT_1000892c[0x16] = local_cc + 1;
    local_dc = (void *)0x0;
    local_d4 = 0;
    local_d8 = 0;
    local_d0 = 0;
    local_c8 = 0x9e;
    local_c4 = 1;
    local_c0 = 0;
    local_b8 = 2;
    FUN_10002500();
    memset(local_bc,0,local_b8 << 4);
    local_4 = 0;
    local_ac = 1;
    local_b4[0] = 0;
    local_a8 = 0;
    FUN_100027b0((void *)0x16,",%u=%u");
    FUN_10003350(puVar1,(int *)&local_dc);
    if (local_dc != (void *)0x0) {
      free(local_dc);
    }
  }
  ExceptionList = local_c;
  return;
}



// ---- FUN_10004a00 @ 10004a00 ----

undefined4 FUN_10004a00(uint param_1)

{
  void *_Dst;
  uint *unaff_ESI;
  
  _Dst = realloc((void *)(-(uint)((uint *)*unaff_ESI != unaff_ESI + 3) & *unaff_ESI),param_1 << 4);
  if (_Dst != (void *)0x0) {
    if ((uint *)*unaff_ESI == unaff_ESI + 3) {
      memcpy(_Dst,(uint *)*unaff_ESI,unaff_ESI[1] << 4);
    }
    *unaff_ESI = (uint)_Dst;
    if (param_1 != 0) {
      unaff_ESI[2] = param_1;
      return 1;
    }
  }
  return 0;
}



// ---- FUN_10004a5f @ 10004a5f ----

void __cdecl
FUN_10004a5f(undefined4 param_1,undefined4 param_2,undefined4 param_3,undefined4 param_4)

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



// ---- FUN_10004ab0 @ 10004ab0 ----

void __fastcall FUN_10004ab0(int param_1,undefined4 param_2)

{
  if (param_1 == DAT_1000828c) {
    return;
  }
  FUN_10005233(param_1,param_2);
  return;
}



// ---- __alloca_probe @ 10004ac0 ----

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



// ---- __local_unwind4 @ 10004aec ----

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
  puStack_24 = &LAB_10004b7c;
  pvStack_28 = ExceptionList;
  local_20 = DAT_1000828c ^ (uint)&pvStack_28;
  ExceptionList = &pvStack_28;
  while( true ) {
    uVar2 = *(uint *)(param_2 + 0xc);
    if ((uVar2 == 0xfffffffe) || ((param_3 != 0xfffffffe && (uVar2 <= param_3)))) break;
    puVar1 = (undefined4 *)((*(uint *)(param_2 + 8) ^ *param_1) + 0x10 + uVar2 * 0xc);
    *(undefined4 *)(param_2 + 0xc) = *puVar1;
    if (puVar1[1] == 0) {
      __NLG_Notify(0x101);
      FUN_100053ec();
    }
  }
  ExceptionList = pvStack_28;
  return;
}



// ---- FUN_10004bc2 @ 10004bc2 ----

void FUN_10004bc2(int param_1)

{
  __local_unwind4(*(uint **)(param_1 + 0x28),*(int *)(param_1 + 0x18),*(uint *)(param_1 + 0x1c));
  return;
}



// ---- _EH4_CallFilterFunc @ 10004bde ----

/* Library Function - Single Match
    @_EH4_CallFilterFunc@8
   
   Library: Visual Studio 2008 Release
   __fastcall _EH4_CallFilterFunc,8 */

void __fastcall _EH4_CallFilterFunc(undefined *param_1)

{
  (*(code *)param_1)();
  return;
}



// ---- _EH4_TransferToHandler @ 10004bf5 ----

/* Library Function - Single Match
    @_EH4_TransferToHandler@8
   
   Library: Visual Studio 2008 Release
   __fastcall _EH4_TransferToHandler,8 */

void __fastcall _EH4_TransferToHandler(undefined *UNRECOVERED_JUMPTABLE)

{
  __NLG_Notify(1);
                    /* WARNING: Could not recover jumptable at 0x10004c0c. Too many branches */
                    /* WARNING: Treating indirect jump as call */
  (*(code *)UNRECOVERED_JUMPTABLE)();
  return;
}



// ---- _EH4_GlobalUnwind @ 10004c0e ----

/* Library Function - Single Match
    @_EH4_GlobalUnwind@4
   
   Library: Visual Studio 2008 Release
   __fastcall _EH4_GlobalUnwind,4 */

void __fastcall _EH4_GlobalUnwind(PVOID param_1)

{
  RtlUnwind(param_1,(PVOID)0x10004c23,(PEXCEPTION_RECORD)0x0,(PVOID)0x0);
  return;
}



// ---- _EH4_LocalUnwind @ 10004c28 ----

/* Library Function - Single Match
    @_EH4_LocalUnwind@16
   
   Library: Visual Studio 2008 Release
   __fastcall _EH4_LocalUnwind,16 */

void __fastcall _EH4_LocalUnwind(int param_1,uint param_2,undefined4 param_3,uint *param_4)

{
  __local_unwind4(param_4,param_1,param_2);
  return;
}



// ---- __except_handler4_common @ 10004c40 ----

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
          goto LAB_10004ce8;
        }
        if (0 < iVar1) {
          if (((*param_3 == -0x1f928c9d) && (DAT_10008964 != (code *)0x0)) &&
             (uVar2 = FUN_10005474(0x10008964), uVar2 != 0)) {
            (*DAT_10008964)(param_3,1);
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
          goto LAB_10004da3;
        }
      }
      pvVar3 = local_c;
    } while (local_c != (PVOID)0xfffffffe);
    if (local_5 == '\0') {
      return local_10;
    }
  }
  else {
LAB_10004da3:
    if (*(int *)((int)pvVar3 + 0xc) == -2) {
      return local_10;
    }
    _EH4_LocalUnwind((int)pvVar3,0xfffffffe,(int)param_4 + 0x10,param_1);
  }
LAB_10004ce8:
  if (*piVar4 != -2) {
    (*(code *)param_2)();
  }
  (*(code *)param_2)();
  return local_10;
}



// ---- FUN_10004dd4 @ 10004dd4 ----

int __cdecl FUN_10004dd4(int param_1,_EXCEPTION_POINTERS *param_2)

{
  int iVar1;
  
  if (param_1 == -0x1f928c9d) {
    iVar1 = _XcptFilter(0xe06d7363,param_2);
    return iVar1;
  }
  return 0;
}



// ---- __initterm_e @ 10004df4 ----

/* Library Function - Single Match
    __initterm_e
   
   Libraries: Visual Studio 2008 Release, Visual Studio 2010 Release, Visual Studio 2012 Release */

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



// ---- FUN_10004e3a @ 10004e3a ----

undefined4 FUN_10004e3a(undefined4 param_1,int param_2,undefined4 param_3)

{
  bool bVar1;
  undefined4 *_Memory;
  int iVar2;
  uint uVar3;
  LONG LVar4;
  undefined4 *puVar5;
  
  if (param_2 == 0) {
    if (DAT_100085ec < 1) {
      return 0;
    }
    DAT_100085ec = DAT_100085ec + -1;
    while (LVar4 = InterlockedCompareExchange((LONG *)&DAT_10008950,1,0), _Memory = DAT_10008958,
          LVar4 != 0) {
      Sleep(1000);
    }
    if (DAT_1000894c == 2) {
      puVar5 = DAT_10008954;
      if (DAT_10008958 != (undefined4 *)0x0) {
        while (puVar5 = puVar5 + -1, _Memory <= puVar5) {
          if ((code *)*puVar5 != (code *)0x0) {
            (*(code *)*puVar5)();
          }
        }
        free(_Memory);
        DAT_10008954 = (undefined4 *)0x0;
        DAT_10008958 = (undefined4 *)0x0;
      }
      DAT_1000894c = 0;
      InterlockedExchange((LONG *)&DAT_10008950,0);
    }
    else {
      _amsg_exit(0x1f);
    }
  }
  else if (param_2 == 1) {
    iVar2 = *(int *)((int)Self + 4);
    bVar1 = false;
    while (LVar4 = InterlockedCompareExchange((LONG *)&DAT_10008950,iVar2,0), LVar4 != 0) {
      if (LVar4 == iVar2) {
        bVar1 = true;
        break;
      }
      Sleep(1000);
    }
    if (DAT_1000894c == 0) {
      DAT_1000894c = 1;
      iVar2 = __initterm_e((undefined4 *)&DAT_10006114,(undefined4 *)&DAT_1000611c);
      if (iVar2 != 0) {
        return 0;
      }
      initterm(&DAT_10006104,&DAT_10006110);
      DAT_1000894c = 2;
    }
    else {
      _amsg_exit(0x1f);
    }
    if (!bVar1) {
      InterlockedExchange((LONG *)&DAT_10008950,0);
    }
    if ((DAT_10008960 != (code *)0x0) && (uVar3 = FUN_10005474(0x10008960), uVar3 != 0)) {
      (*DAT_10008960)(param_1,2,param_3);
    }
    DAT_100085ec = DAT_100085ec + 1;
  }
  return 1;
}



// ---- FUN_10004fc1 @ 10004fc1 ----

/* WARNING: Function: __SEH_prolog4 replaced with injection: SEH_prolog4 */
/* WARNING: Function: __SEH_epilog4 replaced with injection: EH_epilog3 */
/* WARNING: Globals starting with '_' overlap smaller symbols at the same address */

int FUN_10004fc1(undefined4 param_1,int param_2,undefined4 param_3)

{
  int local_20;
  
  local_20 = 1;
  _DAT_100082b0 = param_2;
  if ((param_2 == 0) && (DAT_100085ec == 0)) {
    local_20 = 0;
  }
  else {
    if ((param_2 == 1) || (param_2 == 2)) {
      if (DAT_1000895c != (code *)0x0) {
        DAT_100085f0 = 1;
        local_20 = (*DAT_1000895c)(param_1,param_2,param_3);
      }
      if ((local_20 == 0) || (local_20 = FUN_10004e3a(param_1,param_2,param_3), local_20 == 0))
      goto LAB_100051fb;
    }
    local_20 = FUN_100024b0(param_1,param_2);
    if ((param_2 == 1) && (local_20 == 0)) {
      FUN_100024b0(param_1,0);
      FUN_10004e3a(param_1,0,0);
      if (DAT_1000895c != (code *)0x0) {
        (*DAT_1000895c)(param_1,0,0);
      }
    }
    if ((((param_2 == 0) || (param_2 == 3)) &&
        (local_20 = FUN_10004e3a(param_1,param_2,param_3), DAT_1000895c != (code *)0x0)) &&
       (DAT_100085f0 != 0)) {
      local_20 = (*DAT_1000895c)(param_1,param_2,param_3);
    }
  }
LAB_100051fb:
  FUN_10005212();
  return local_20;
}



// ---- FUN_10005212 @ 10005212 ----

/* WARNING: Globals starting with '_' overlap smaller symbols at the same address */

void FUN_10005212(void)

{
  _DAT_100082b0 = 0xffffffff;
  return;
}



// ---- entry @ 1000521d ----

void entry(undefined4 param_1,int param_2,undefined4 param_3)

{
  if (param_2 == 1) {
    FUN_100055a6();
  }
  FUN_10004fc1(param_1,param_2,param_3);
  return;
}



// ---- FUN_10005233 @ 10005233 ----

/* WARNING: Globals starting with '_' overlap smaller symbols at the same address */

void __fastcall FUN_10005233(undefined4 param_1,undefined4 param_2)

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
  
  _DAT_10008708 =
       (uint)(in_NT & 1) * 0x4000 | (uint)SBORROW4((int)&stack0xfffffffc,0x328) * 0x800 |
       (uint)(in_IF & 1) * 0x200 | (uint)(in_TF & 1) * 0x100 | (uint)((int)&local_32c < 0) * 0x80 |
       (uint)(&stack0x00000000 == (undefined1 *)0x32c) * 0x40 | (uint)(in_AF & 1) * 0x10 |
       (uint)((POPCOUNT((uint)&local_32c & 0xff) & 1U) == 0) * 4 |
       (uint)(&stack0xfffffffc < (undefined1 *)0x328) | (uint)(in_ID & 1) * 0x200000 |
       (uint)(in_VIP & 1) * 0x100000 | (uint)(in_VIF & 1) * 0x80000 | (uint)(in_AC & 1) * 0x40000;
  _DAT_1000870c = &stack0x00000004;
  _DAT_10008648 = 0x10001;
  _DAT_100085f8 = 0xc0000409;
  _DAT_100085fc = 1;
  local_32c = DAT_1000828c;
  local_328 = DAT_10008290;
  _DAT_10008604 = unaff_retaddr;
  _DAT_100086d4 = in_GS;
  _DAT_100086d8 = in_FS;
  _DAT_100086dc = in_ES;
  _DAT_100086e0 = in_DS;
  _DAT_100086e4 = unaff_EDI;
  _DAT_100086e8 = unaff_ESI;
  _DAT_100086ec = unaff_EBX;
  _DAT_100086f0 = param_2;
  _DAT_100086f4 = param_1;
  _DAT_100086f8 = in_EAX;
  _DAT_100086fc = unaff_EBP;
  DAT_10008700 = unaff_retaddr;
  _DAT_10008704 = in_CS;
  _DAT_10008710 = in_SS;
  SetUnhandledExceptionFilter((LPTOP_LEVEL_EXCEPTION_FILTER)0x0);
  UnhandledExceptionFilter((_EXCEPTION_POINTERS *)&PTR_DAT_1000694c);
  uExitCode = 0xc0000409;
  hProcess = GetCurrentProcess();
  TerminateProcess(hProcess,uExitCode);
  return;
}



// ---- __NLG_Notify @ 100053cd ----

/* Library Function - Single Match
    __NLG_Notify
   
   Libraries: Visual Studio 2017 Debug, Visual Studio 2017 Release, Visual Studio 2019 Debug, Visual
   Studio 2019 Release */

void __NLG_Notify(ulong param_1)

{
  undefined4 in_EAX;
  undefined4 unaff_EBP;
  
  DAT_100082a8 = param_1;
  DAT_100082a4 = in_EAX;
  DAT_100082ac = unaff_EBP;
  return;
}



// ---- FUN_100053ec @ 100053ec ----

void FUN_100053ec(void)

{
  code *in_EAX;
  
  (*in_EAX)();
  return;
}



// ---- __ValidateImageBase @ 100053f0 ----

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



// ---- __FindPESection @ 10005430 ----

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



// ---- FUN_10005474 @ 10005474 ----

/* WARNING: Function: __SEH_prolog4 replaced with injection: SEH_prolog4 */
/* WARNING: Function: __SEH_epilog4 replaced with injection: EH_epilog3 */

uint __cdecl FUN_10005474(int param_1)

{
  BOOL BVar1;
  PIMAGE_SECTION_HEADER p_Var2;
  
  BVar1 = __ValidateImageBase((PBYTE)&IMAGE_DOS_HEADER_10000000);
  if ((BVar1 != 0) &&
     (p_Var2 = __FindPESection((PBYTE)&IMAGE_DOS_HEADER_10000000,param_1 + 0xf0000000),
     p_Var2 != (PIMAGE_SECTION_HEADER)0x0)) {
    return ~(p_Var2->Characteristics >> 0x1f) & 1;
  }
  return 0;
}



// ---- __SEH_prolog4 @ 10005528 ----

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
  *(uint *)((int)auStack_1c + iVar1 + 4) = DAT_1000828c ^ (uint)&param_2;
  *(undefined4 *)((int)auStack_1c + iVar1) = unaff_retaddr;
  ExceptionList = local_8;
  return;
}



// ---- __SEH_epilog4 @ 1000556d ----

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



// ---- __except_handler4 @ 10005581 ----

/* Library Function - Single Match
    __except_handler4
   
   Library: Visual Studio 2008 Release */

void __cdecl __except_handler4(int *param_1,PVOID param_2,undefined4 param_3)

{
  __except_handler4_common(&DAT_1000828c,FUN_10004ab0,param_1,param_2,param_3);
  return;
}



// ---- FUN_100055a6 @ 100055a6 ----

void FUN_100055a6(void)

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
  if ((DAT_1000828c == 0xbb40e64e) || ((DAT_1000828c & 0xffff0000) == 0)) {
    GetSystemTimeAsFileTime(&local_c);
    uVar5 = local_c.dwHighDateTime ^ local_c.dwLowDateTime;
    DVar2 = GetCurrentProcessId();
    DVar3 = GetCurrentThreadId();
    DVar4 = GetTickCount();
    QueryPerformanceCounter(&local_14);
    uVar5 = uVar5 ^ DVar2 ^ DVar3 ^ DVar4 ^ local_14.s.HighPart ^ local_14.s.LowPart;
    if ((uVar5 == 0xbb40e64e) ||
       (uVar1 = DAT_1000828c & 0xffff0000, DAT_1000828c = uVar5, uVar1 == 0)) {
      DAT_1000828c = 0xbb40e64f;
    }
  }
  DAT_10008290 = ~DAT_1000828c;
  return;
}



// ---- Unwind@10005640 @ 10005640 ----

void Unwind_10005640(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@10005670 @ 10005670 ----

void Unwind_10005670(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@1000567b @ 1000567b ----

void Unwind_1000567b(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@100056b0 @ 100056b0 ----

void Unwind_100056b0(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@100056bb @ 100056bb ----

void Unwind_100056bb(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@100056c6 @ 100056c6 ----

void Unwind_100056c6(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@100056d1 @ 100056d1 ----

void Unwind_100056d1(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@100056dc @ 100056dc ----

void Unwind_100056dc(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@100056e7 @ 100056e7 ----

void Unwind_100056e7(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@100056f2 @ 100056f2 ----

void Unwind_100056f2(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@10005720 @ 10005720 ----

void Unwind_10005720(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@1000572b @ 1000572b ----

void Unwind_1000572b(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@10005736 @ 10005736 ----

void Unwind_10005736(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@10005741 @ 10005741 ----

void Unwind_10005741(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@1000574c @ 1000574c ----

void Unwind_1000574c(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@10005757 @ 10005757 ----

void Unwind_10005757(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@10005780 @ 10005780 ----

void Unwind_10005780(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@100057b0 @ 100057b0 ----

void Unwind_100057b0(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@100057b8 @ 100057b8 ----

void Unwind_100057b8(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@100057e0 @ 100057e0 ----

void Unwind_100057e0(void)

{
  FUN_100025c0();
  return;
}



// ---- Unwind@10005810 @ 10005810 ----

void Unwind_10005810(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@10005818 @ 10005818 ----

void Unwind_10005818(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@10005820 @ 10005820 ----

void Unwind_10005820(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@10005828 @ 10005828 ----

void Unwind_10005828(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@10005830 @ 10005830 ----

void Unwind_10005830(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@10005838 @ 10005838 ----

void Unwind_10005838(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@10005840 @ 10005840 ----

void Unwind_10005840(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@10005848 @ 10005848 ----

void Unwind_10005848(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@10005880 @ 10005880 ----

void Unwind_10005880(void)

{
  FUN_10001150();
  return;
}



// ---- Unwind@100058b0 @ 100058b0 ----

void Unwind_100058b0(void)

{
  FUN_100025c0();
  return;
}



// ---- Unwind@100058e0 @ 100058e0 ----

void Unwind_100058e0(void)

{
  FUN_100025c0();
  return;
}



// ---- Unwind@10005910 @ 10005910 ----

void Unwind_10005910(void)

{
  FUN_100025c0();
  return;
}



// ---- Unwind@10005950 @ 10005950 ----

void Unwind_10005950(void)

{
  FUN_100025c0();
  return;
}


