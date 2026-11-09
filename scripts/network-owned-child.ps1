$ErrorActionPreference = 'Stop'
# Suspended creation closes the create/assign race. Closing this wrapper's Job handle
# terminates descendants even when the direct child exited with inherited pipes open.
$definition = @'
using System;
using System.IO;
using System.Text;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class FluxDLNetworkOwnedChild {
 [StructLayout(LayoutKind.Sequential)] struct SECURITY_ATTRIBUTES { public int length; public IntPtr descriptor; public int inherit; }
 [StructLayout(LayoutKind.Sequential,CharSet=CharSet.Unicode)] struct STARTUPINFO { public int size; public string reserved; public string desktop; public string title; public uint x,y,xSize,ySize,xChars,yChars,fill,flags; public ushort show,reservedSize; public IntPtr reserved2,input,output,error; }
 [StructLayout(LayoutKind.Sequential)] struct PROCESS_INFORMATION { public IntPtr process,thread; public uint processId,threadId; }
 [StructLayout(LayoutKind.Sequential)] struct BASIC_LIMIT { public long processTime,jobTime; public uint flags; public UIntPtr minWorking,maxWorking; public uint active; public UIntPtr affinity; public uint priority,scheduling; }
 [StructLayout(LayoutKind.Sequential)] struct IO_COUNTERS { public ulong readOps,writeOps,otherOps,readBytes,writeBytes,otherBytes; }
 [StructLayout(LayoutKind.Sequential)] struct EXTENDED_LIMIT { public BASIC_LIMIT basic; public IO_COUNTERS io; public UIntPtr processMemory,jobMemory,peakProcess,peakJob; }
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr CreateJobObject(IntPtr attributes,string name);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool SetInformationJobObject(IntPtr job,int info,ref EXTENDED_LIMIT limits,uint length);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool AssignProcessToJobObject(IntPtr job,IntPtr process);
 [DllImport("kernel32.dll")] static extern bool TerminateJobObject(IntPtr job,uint code);
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool CreateProcess(string app,StringBuilder cmd,IntPtr processAttributes,IntPtr threadAttributes,bool inherit,uint flags,IntPtr environment,string directory,ref STARTUPINFO startup,out PROCESS_INFORMATION process);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool CreatePipe(out IntPtr read,out IntPtr write,ref SECURITY_ATTRIBUTES attributes,uint size);
 [DllImport("kernel32.dll")] static extern bool SetHandleInformation(IntPtr handle,uint mask,uint flags);
 [DllImport("kernel32.dll")] static extern IntPtr GetStdHandle(int id);
 [DllImport("kernel32.dll")] static extern uint ResumeThread(IntPtr thread);
 [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr handle,uint timeout);
 [DllImport("kernel32.dll")] static extern bool GetExitCodeProcess(IntPtr process,out uint code);
 [DllImport("kernel32.dll")] static extern bool TerminateProcess(IntPtr process,uint code);
 [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
 static string Quote(string value) {
  if(value.Length>0 && value.IndexOfAny(new char[]{' ','\t','\n','\v','"'})<0)return value;
  var b=new StringBuilder("\""); int slashes=0;
  foreach(char c in value){if(c=='\\'){slashes++;continue;} if(c=='"'){b.Append('\\',slashes*2+1);b.Append('"');}else{b.Append('\\',slashes);b.Append(c);}slashes=0;}
  b.Append('\\',slashes*2);b.Append('"');return b.ToString();
 }
 public static int Run(string executable,string[] args,string directory,string input,int timeoutMs) {
  IntPtr job=IntPtr.Zero,read=IntPtr.Zero,write=IntPtr.Zero; PROCESS_INFORMATION pi=new PROCESS_INFORMATION();
  try {
   job=CreateJobObject(IntPtr.Zero,null);if(job==IntPtr.Zero)throw new Exception();
   var limits=new EXTENDED_LIMIT();limits.basic.flags=0x2000;
   if(!SetInformationJobObject(job,9,ref limits,(uint)Marshal.SizeOf(typeof(EXTENDED_LIMIT))))throw new Exception();
   var security=new SECURITY_ATTRIBUTES();security.length=Marshal.SizeOf(typeof(SECURITY_ATTRIBUTES));security.inherit=1;
   if(!CreatePipe(out read,out write,ref security,0)||!SetHandleInformation(write,1,0))throw new Exception();
   var startup=new STARTUPINFO();startup.size=Marshal.SizeOf(typeof(STARTUPINFO));startup.flags=0x100;startup.input=read;startup.output=GetStdHandle(-11);startup.error=GetStdHandle(-12);
   var cmd=new StringBuilder(Quote(executable));foreach(string arg in args)cmd.Append(" ").Append(Quote(arg));
   if(!CreateProcess(executable,cmd,IntPtr.Zero,IntPtr.Zero,true,0x08000004,IntPtr.Zero,directory,ref startup,out pi))throw new Exception();
   CloseHandle(read);read=IntPtr.Zero;
   if(!AssignProcessToJobObject(job,pi.process)){TerminateProcess(pi.process,125);throw new Exception();}
   if(ResumeThread(pi.thread)==0xffffffff){TerminateJobObject(job,125);throw new Exception();}
   using(var stream=new FileStream(new SafeFileHandle(write,true),FileAccess.Write)){write=IntPtr.Zero;byte[] bytes=Encoding.UTF8.GetBytes(input);stream.Write(bytes,0,bytes.Length);}
   if(WaitForSingleObject(pi.process,(uint)timeoutMs)!=0){TerminateJobObject(job,124);WaitForSingleObject(pi.process,10000);return 124;}
   uint code;return GetExitCodeProcess(pi.process,out code)?unchecked((int)code):125;
  }catch{if(job!=IntPtr.Zero)TerminateJobObject(job,125);Console.Error.WriteLine("owned_process_failure");return 125;}
  finally{if(job!=IntPtr.Zero)CloseHandle(job);if(read!=IntPtr.Zero)CloseHandle(read);if(write!=IntPtr.Zero)CloseHandle(write);if(pi.thread!=IntPtr.Zero)CloseHandle(pi.thread);if(pi.process!=IntPtr.Zero)CloseHandle(pi.process);}
 }
}
'@
try {
    Add-Type -TypeDefinition $definition -Language CSharp
    $request = [Console]::In.ReadToEnd() | ConvertFrom-Json
    if (-not [IO.Path]::IsPathRooted([string]$request.executable) -or -not [IO.Path]::IsPathRooted([string]$request.directory) -or [int]$request.timeoutMs -lt 50 -or [int]$request.timeoutMs -gt 60000) { exit 125 }
    $code = [FluxDLNetworkOwnedChild]::Run([string]$request.executable,[string[]]$request.args,[string]$request.directory,[string]$request.input,[int]$request.timeoutMs)
    exit $code
} catch { [Console]::Error.WriteLine('owned_process_wrapper_failure'); exit 125 }
