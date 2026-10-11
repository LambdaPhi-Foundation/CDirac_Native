// consts.cdc -- constants + static data members as first-class Dirac.
//
// run:
//   cdirac --emit-llvm examples\cpp\consts.cdc > out.ll
//   cdirac --emit-cpp-shim examples\cpp\consts.cdc > cdi_shim.cpp
//   clang++ out.ll llvmrt\*.c cdi_shim.cpp -I llvmrt -I examples\cpp -o consts
//   ./consts
// expect: 1, 6, 1, 64, dirac, 3.14159, true, 0, 1, v2, 10

[include-cpp "consts.h"]
[cpp-enum RED "dirac_demo::RED" GREEN "dirac_demo::GREEN" BLUE "dirac_demo::BLUE"]
[cpp-enum MODE_ON "dirac_demo::Mode::ON"]
[cpp-const MAX_ITEMS "MAX_ITEMS" int]
[cpp-const APP_NAME "APP_NAME" string]
[cpp-const PI_D "dirac_demo::PI_D" double]
[cpp-const VERBOSE "dirac_demo::VERBOSE" bool]
[cpp-class Config "dirac_demo::Config" "consts.h"]
[cpp-static Config bump [] void]
[cpp-static-field Config instances int]
[cpp-static-field Config tag string]
[cpp-static-field Config LIMIT int const]

displayln(RED);
displayln(GREEN + BLUE);
displayln(MODE_ON);
displayln(MAX_ITEMS);
displayln(APP_NAME);
displayln(PI_D);
displayln(VERBOSE);
displayln(Config.instances);
Config.bump();
displayln(Config.instances);
set Config.tag = "v2";
displayln(Config.tag);
displayln(Config.LIMIT);
