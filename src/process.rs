// process 模块预留
// 当前 Java 进程 spawn 逻辑在 instance::InstanceManager::create 中
// 后续可扩展为 trait ConsoleBackend：
//   - PipeBackend (当前实现)
//   - PtyBackend
//   - RconBackend
