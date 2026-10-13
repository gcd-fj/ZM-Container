package {
    import flash.display.MovieClip;
    import flash.utils.ByteArray;
    import flash.utils.Endian;
    public class ByteArrayRoundTrip extends MovieClip {
        private function check(ok:Boolean, label:String):void {
            if (!ok) throw new Error(label);
        }
        public function ByteArrayRoundTrip() {
            for each (var algorithm:String in ["zlib", "deflate", "lzma"]) {
                for each (var size:int in [0, 1, 256, 65537]) {
                    var bytes:ByteArray = new ByteArray();
                    bytes.endian = Endian.LITTLE_ENDIAN;
                    bytes.objectEncoding = 0;
                    for (var i:int = 0; i < size; ++i) bytes.writeByte((i * 73) & 255);
                    bytes.position = size / 2;
                    bytes.compress(algorithm);
                    check(bytes.position == bytes.length, "compressed position");
                    check(bytes.endian == Endian.LITTLE_ENDIAN && bytes.objectEncoding == 0, "compressed settings");
                    bytes.position = bytes.length;
                    bytes.uncompress(algorithm);
                    check(bytes.position == 0 && bytes.length == size, "uncompressed position/length");
                    check(bytes.endian == Endian.LITTLE_ENDIAN && bytes.objectEncoding == 0, "uncompressed settings");
                    for (i = 0; i < size; ++i) check(bytes.readUnsignedByte() == ((i * 73) & 255), "roundtrip data");
                    // A second cycle checks the adopted buffer can be reused and extended.
                    bytes.writeByte(42);
                    bytes.compress(algorithm);
                    bytes.uncompress(algorithm);
                    check(bytes.length == size + 1 && bytes[size] == 42, "second cycle");
                }
            }
            var invalid:ByteArray = new ByteArray();
            invalid.writeUnsignedInt(0xffffffff);
            invalid.position = 2;
            var errorId:int = 0;
            try { invalid.uncompress("zlib"); } catch (e:Error) { errorId = e.errorID; }
            check(errorId == 2058, "invalid compressed data error");
            check(invalid.length == 4 && invalid.position == 2 && invalid[0] == 255, "error preserves buffer");
            errorId = 0;
            try { invalid.uncompress("unknown"); } catch (e2:Error) { errorId = e2.errorID; }
            check(errorId == 2058 && invalid.position == 2 && invalid.length == 4, "unknown algorithm");
            trace("BYTEARRAY_ROUNDTRIP_OK");
        }
    }
}
