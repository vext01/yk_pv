%%
@[a-zA-Z_.]+ "GLOBAL"
%[0-9]+ "LOCAL_OPERAND"
i[0-9]+ "INT_TYPE"
[0-9]+ "INT"
load_ti "LOAD_TI"
test_use "TEST_USE"
add "ADD"
[a-zA_Z_]+: "LABEL"
[a-zA_Z_]+ "ID"
: ":"
, ","
= "="
;.*?$ ;
[\t \n\r]+ ;