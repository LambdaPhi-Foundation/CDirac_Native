// enum_const.cdc — enums and named constants read as values.
[include-cpp "consts.h"]
[cpp-enum RED "ns::Color::RED" GREEN "ns::Color::GREEN"]
[cpp-const MAX_ITEMS "MAX_ITEMS" int]
[cpp-const APP_NAME "APP_NAME" string]
displayln(RED);
displayln(GREEN);
displayln(MAX_ITEMS);
displayln(APP_NAME);
