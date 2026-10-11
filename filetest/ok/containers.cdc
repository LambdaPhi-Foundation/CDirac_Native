// containers.cdc — arrays and queues.
var a = array(1, 2, 3);
displayln(len(a));
displayln(get(a, 0));
put(a, 0, 99);
push(a, 4);
displayln(len(a));
var q = queue(10);
qpush(q, 20);
displayln(qpop(q));
