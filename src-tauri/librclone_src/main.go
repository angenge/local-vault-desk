package main

/*
#include <stdlib.h>

struct RcloneRPCResult {
	char*	Output;
	int	Status;
};
*/
import "C"

import (
	"unsafe"

	"github.com/rclone/rclone/fs/config"
	"github.com/rclone/rclone/librclone/librclone"

	_ "github.com/rclone/rclone/backend/crypt"
	_ "github.com/rclone/rclone/backend/local"
	_ "github.com/rclone/rclone/fs/operations"
	_ "github.com/rclone/rclone/fs/rc/rcflags"
	_ "github.com/rclone/rclone/fs/rc/rcserver"
	_ "github.com/rclone/rclone/fs/sync"
)

func init() {
	// 强制纯内存配置：configPath="" 时 configfile.Install() 的 SetData 会保留内存版
	// defaultStorage（Load/Save 均为 no-op），杜绝共享 %APPDATA%\rclone\rclone.conf
	// 被多进程互相覆盖与自动重载导致的 "didn't find section in config file" 竞态，
	// 同时满足"零配置文件落盘"的设计约束。
	_ = config.SetConfigPath("")
}

//export RcloneInitialize
func RcloneInitialize() {
	librclone.Initialize()
}

//export RcloneFinalize
func RcloneFinalize() {
	librclone.Finalize()
}

//export RcloneRPC
func RcloneRPC(method *C.char, input *C.char) (result C.struct_RcloneRPCResult) {
	output, status := librclone.RPC(C.GoString(method), C.GoString(input))
	result.Output = C.CString(output)
	result.Status = C.int(status)
	return result
}

//export RcloneFreeString
func RcloneFreeString(str *C.char) {
	C.free(unsafe.Pointer(str))
}

func main() {}
