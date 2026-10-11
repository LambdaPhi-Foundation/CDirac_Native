// machine_mem.cdc — raw machine allocation round-trips a handle.
var h = machine.alloc(16);
machine.dealloc(h);
displayln("ok");
