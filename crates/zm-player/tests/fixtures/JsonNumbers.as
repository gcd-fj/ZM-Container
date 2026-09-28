package {
    import flash.display.MovieClip;
    public class JsonNumbers extends MovieClip {
        public function JsonNumbers() {
            var values:Array = [1788615600000, 2147483648, 4294967295, 4294967296,
                -2147483649, 9007199254740991, -9007199254740991, 7, 0, 1.5];
            for each (var expected:Number in values) {
                var decoded:Object = JSON.parse('{"time":' + expected + '}');
                if (decoded.time !== expected) {
                    trace("JSON_NUMBER_FAIL " + expected + " -> " + decoded.time);
                    return;
                }
            }
            var date:Date = new Date(JSON.parse("1788615600000"));
            if (date.fullYearUTC != 2026 || date.dayUTC != 6) {
                trace("JSON_DATE_FAIL");
                return;
            }
            var dates:Array = ["2026/9/5 0:00", "2026/09/05 00:00:00", "9/5/2026 0:00:00"];
            var reference:Number = new Date(2026, 8, 5).time;
            for each (var text:String in dates) {
                if (new Date(text).time !== reference) {
                    trace("ACTIVITY_DATE_FAIL " + text);
                    return;
                }
            }
            var calls:Array = [];
            var nested:Object = JSON.parse('{"outer":[{"keep":7,"drop":1},null,"中文"],"flag":true}',
                function(key:*, value:*):* {
                    calls.push(String(key));
                    if (key === "drop") return undefined;
                    if (key === "keep") return value + 1;
                    return value;
                });
            if (nested.outer[0].keep !== 8 || nested.outer[0].hasOwnProperty("drop") ||
                nested.outer[1] !== null || nested.outer[2] !== "中文" || nested.flag !== true ||
                calls.join(",") !== "keep,drop,0,1,2,outer,flag,") {
                trace("JSON_NESTED_REVIVER_FAIL");
                return;
            }
            var deep:String = '{"value":1788615600000}';
            for (var i:int = 0; i < 48; i++) deep = '{"child":[' + deep + ']}';
            var node:Object = JSON.parse(deep);
            for (i = 0; i < 48; i++) node = node.child[0];
            if (node.value !== 1788615600000) {
                trace("JSON_DEEP_TREE_FAIL");
                return;
            }
            trace("JSON_NUMBERS_OK");
        }
    }
}
