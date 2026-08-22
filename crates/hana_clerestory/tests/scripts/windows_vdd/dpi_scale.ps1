# Query / set a monitor's per-monitor DPI scale via the undocumented
# DisplayConfigSetDeviceInfo SET_SOURCE_DPI_SCALE (type -4) API. Targets the source whose
# target monitor device path contains -Match (e.g. "MTT1337", the VDD). Windows-only.
# All struct work is done in C# (PowerShell mishandles nested value-type field writes).
#
#   dpi_scale.ps1 -Match MTT1337 -Action list
#   dpi_scale.ps1 -Match MTT1337 -Action get
#   dpi_scale.ps1 -Match MTT1337 -Action setmin        # 100%
#   dpi_scale.ps1 -Match MTT1337 -Action setrel -Rel 0 # back to recommended
param(
  [Parameter(Mandatory)] [string]$Match,
  [ValidateSet('get','setmin','setrel','list')] [string]$Action = 'get',
  [int]$Rel = 0
)
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Runtime.InteropServices;

public static class Dpi {
    [StructLayout(LayoutKind.Sequential)] public struct LUID { public uint LowPart; public int HighPart; }
    [StructLayout(LayoutKind.Sequential)] public struct PATH_SOURCE_INFO { public LUID adapterId; public uint id; public uint modeInfoIdx; public uint statusFlags; }
    [StructLayout(LayoutKind.Sequential)] public struct PATH_TARGET_INFO {
        public LUID adapterId; public uint id; public uint modeInfoIdx;
        public uint outputTechnology; public uint rotation; public uint scaling;
        public uint refreshNum; public uint refreshDen; public uint scanLineOrdering;
        public int targetAvailable; public uint statusFlags;
    }
    [StructLayout(LayoutKind.Sequential)] public struct PATH_INFO { public PATH_SOURCE_INFO sourceInfo; public PATH_TARGET_INFO targetInfo; public uint flags; }
    [StructLayout(LayoutKind.Sequential)] public struct MODE_INFO { public uint infoType; public uint id; public LUID adapterId; public ulong u0,u1,u2,u3,u4,u5; }
    [StructLayout(LayoutKind.Sequential)] public struct DEVICE_INFO_HEADER { public uint type; public uint size; public LUID adapterId; public uint id; }
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct TARGET_DEVICE_NAME {
        public DEVICE_INFO_HEADER header;
        public uint flags; public uint outputTechnology;
        public ushort edidManufactureId; public ushort edidProductCodeId; public uint connectorInstance;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=64)] public string monitorFriendlyDeviceName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string monitorDevicePath;
    }
    [StructLayout(LayoutKind.Sequential)] public struct SOURCE_DPI_SCALE_GET { public DEVICE_INFO_HEADER header; public int minScaleRel; public int curScaleRel; public int maxScaleRel; }
    [StructLayout(LayoutKind.Sequential)] public struct SOURCE_DPI_SCALE_SET { public DEVICE_INFO_HEADER header; public int scaleRel; }

    [DllImport("user32.dll")] static extern int GetDisplayConfigBufferSizes(uint flags, out uint numPaths, out uint numModes);
    [DllImport("user32.dll")] static extern int QueryDisplayConfig(uint flags, ref uint numPaths, [Out] PATH_INFO[] paths, ref uint numModes, [Out] MODE_INFO[] modes, IntPtr topo);
    [DllImport("user32.dll")] static extern int DisplayConfigGetDeviceInfo(ref TARGET_DEVICE_NAME req);
    [DllImport("user32.dll")] static extern int DisplayConfigGetDeviceInfo(ref SOURCE_DPI_SCALE_GET req);
    [DllImport("user32.dll")] static extern int DisplayConfigSetDeviceInfo(ref SOURCE_DPI_SCALE_SET req);

    const uint QDC_ONLY_ACTIVE_PATHS = 2;
    const uint GET_TARGET_NAME = 2;
    const uint GET_DPI = 0xFFFFFFFD; // -3
    const uint SET_DPI = 0xFFFFFFFC; // -4

    public static string Run(string match, string action, int rel) {
        var sb = new StringBuilder();
        uint np, nm;
        int rc = GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, out np, out nm);
        if (rc != 0) return "GetDisplayConfigBufferSizes rc=" + rc;
        var paths = new PATH_INFO[np];
        var modes = new MODE_INFO[nm];
        rc = QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, ref np, paths, ref nm, modes, IntPtr.Zero);
        if (rc != 0) return "QueryDisplayConfig rc=" + rc;
        sb.AppendLine("paths=" + np);
        bool haveSrc = false; PATH_SOURCE_INFO src = new PATH_SOURCE_INFO();
        for (uint i = 0; i < np; i++) {
            var tn = new TARGET_DEVICE_NAME();
            tn.header.type = GET_TARGET_NAME;
            tn.header.size = (uint)Marshal.SizeOf(typeof(TARGET_DEVICE_NAME));
            tn.header.adapterId = paths[i].targetInfo.adapterId;
            tn.header.id = paths[i].targetInfo.id;
            int r2 = DisplayConfigGetDeviceInfo(ref tn);
            sb.AppendLine("path[" + i + "] rc=" + r2 + " srcId=" + paths[i].sourceInfo.id +
                " friendly='" + tn.monitorFriendlyDeviceName + "' dev='" + tn.monitorDevicePath + "'");
            if (r2 == 0 && !haveSrc && tn.monitorDevicePath != null &&
                (tn.monitorDevicePath.IndexOf(match, StringComparison.OrdinalIgnoreCase) >= 0 ||
                 (tn.monitorFriendlyDeviceName != null && tn.monitorFriendlyDeviceName.IndexOf(match, StringComparison.OrdinalIgnoreCase) >= 0))) {
                src = paths[i].sourceInfo; haveSrc = true;
                sb.AppendLine("matched path[" + i + "] srcId=" + src.id);
            }
        }
        if (action == "list") return sb.ToString();
        if (!haveSrc) return sb.ToString() + "NO MATCH for '" + match + "'\n";

        var g = new SOURCE_DPI_SCALE_GET();
        g.header.type = GET_DPI; g.header.size = (uint)Marshal.SizeOf(typeof(SOURCE_DPI_SCALE_GET));
        g.header.adapterId = src.adapterId; g.header.id = src.id;
        rc = DisplayConfigGetDeviceInfo(ref g);
        sb.AppendLine("GET rc=" + rc + " min=" + g.minScaleRel + " cur=" + g.curScaleRel + " max=" + g.maxScaleRel);
        if (rc != 0 || action == "get") return sb.ToString();

        int target = action == "setmin" ? g.minScaleRel : rel;
        var s = new SOURCE_DPI_SCALE_SET();
        s.header.type = SET_DPI; s.header.size = (uint)Marshal.SizeOf(typeof(SOURCE_DPI_SCALE_SET));
        s.header.adapterId = src.adapterId; s.header.id = src.id; s.scaleRel = target;
        rc = DisplayConfigSetDeviceInfo(ref s);
        sb.AppendLine("SET scaleRel=" + target + " rc=" + rc);
        return sb.ToString();
    }
}
"@

[Dpi]::Run($Match, $Action, $Rel)
