// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'frb_generated.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$EventListenerImplementor {

 BindingEventListener get field0;
/// Create a copy of EventListenerImplementor
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$EventListenerImplementorCopyWith<EventListenerImplementor> get copyWith => _$EventListenerImplementorCopyWithImpl<EventListenerImplementor>(this as EventListenerImplementor, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is EventListenerImplementor&&(identical(other.field0, field0) || other.field0 == field0));
}


@override
int get hashCode => Object.hash(runtimeType,field0);

@override
String toString() {
  return 'EventListenerImplementor(field0: $field0)';
}


}

/// @nodoc
abstract mixin class $EventListenerImplementorCopyWith<$Res>  {
  factory $EventListenerImplementorCopyWith(EventListenerImplementor value, $Res Function(EventListenerImplementor) _then) = _$EventListenerImplementorCopyWithImpl;
@useResult
$Res call({
 BindingEventListener field0
});




}
/// @nodoc
class _$EventListenerImplementorCopyWithImpl<$Res>
    implements $EventListenerImplementorCopyWith<$Res> {
  _$EventListenerImplementorCopyWithImpl(this._self, this._then);

  final EventListenerImplementor _self;
  final $Res Function(EventListenerImplementor) _then;

/// Create a copy of EventListenerImplementor
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') @override $Res call({Object? field0 = null,}) {
  return _then(_self.copyWith(
field0: null == field0 ? _self.field0 : field0 // ignore: cast_nullable_to_non_nullable
as BindingEventListener,
  ));
}

}


/// Adds pattern-matching-related methods to [EventListenerImplementor].
extension EventListenerImplementorPatterns on EventListenerImplementor {
/// A variant of `map` that fallback to returning `orElse`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( EventListenerImplementor_Variant0 value)?  variant0,required TResult orElse(),}){
final _that = this;
switch (_that) {
case EventListenerImplementor_Variant0() when variant0 != null:
return variant0(_that);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// Callbacks receives the raw object, upcasted.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case final Subclass2 value:
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( EventListenerImplementor_Variant0 value)  variant0,}){
final _that = this;
switch (_that) {
case EventListenerImplementor_Variant0():
return variant0(_that);}
}
/// A variant of `map` that fallback to returning `null`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( EventListenerImplementor_Variant0 value)?  variant0,}){
final _that = this;
switch (_that) {
case EventListenerImplementor_Variant0() when variant0 != null:
return variant0(_that);case _:
  return null;

}
}
/// A variant of `when` that fallback to an `orElse` callback.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( BindingEventListener field0)?  variant0,required TResult orElse(),}) {final _that = this;
switch (_that) {
case EventListenerImplementor_Variant0() when variant0 != null:
return variant0(_that.field0);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// As opposed to `map`, this offers destructuring.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case Subclass2(:final field2):
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( BindingEventListener field0)  variant0,}) {final _that = this;
switch (_that) {
case EventListenerImplementor_Variant0():
return variant0(_that.field0);}
}
/// A variant of `when` that fallback to returning `null`
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( BindingEventListener field0)?  variant0,}) {final _that = this;
switch (_that) {
case EventListenerImplementor_Variant0() when variant0 != null:
return variant0(_that.field0);case _:
  return null;

}
}

}

/// @nodoc


class EventListenerImplementor_Variant0 extends EventListenerImplementor {
  const EventListenerImplementor_Variant0(this.field0): super._();
  

@override final  BindingEventListener field0;

/// Create a copy of EventListenerImplementor
/// with the given fields replaced by the non-null parameter values.
@override @JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$EventListenerImplementor_Variant0CopyWith<EventListenerImplementor_Variant0> get copyWith => _$EventListenerImplementor_Variant0CopyWithImpl<EventListenerImplementor_Variant0>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is EventListenerImplementor_Variant0&&(identical(other.field0, field0) || other.field0 == field0));
}


@override
int get hashCode => Object.hash(runtimeType,field0);

@override
String toString() {
  return 'EventListenerImplementor.variant0(field0: $field0)';
}


}

/// @nodoc
abstract mixin class $EventListenerImplementor_Variant0CopyWith<$Res> implements $EventListenerImplementorCopyWith<$Res> {
  factory $EventListenerImplementor_Variant0CopyWith(EventListenerImplementor_Variant0 value, $Res Function(EventListenerImplementor_Variant0) _then) = _$EventListenerImplementor_Variant0CopyWithImpl;
@override @useResult
$Res call({
 BindingEventListener field0
});




}
/// @nodoc
class _$EventListenerImplementor_Variant0CopyWithImpl<$Res>
    implements $EventListenerImplementor_Variant0CopyWith<$Res> {
  _$EventListenerImplementor_Variant0CopyWithImpl(this._self, this._then);

  final EventListenerImplementor_Variant0 _self;
  final $Res Function(EventListenerImplementor_Variant0) _then;

/// Create a copy of EventListenerImplementor
/// with the given fields replaced by the non-null parameter values.
@override @pragma('vm:prefer-inline') $Res call({Object? field0 = null,}) {
  return _then(EventListenerImplementor_Variant0(
null == field0 ? _self.field0 : field0 // ignore: cast_nullable_to_non_nullable
as BindingEventListener,
  ));
}


}

/// @nodoc
mixin _$LoggerImplementor {

 BindingLogger get field0;
/// Create a copy of LoggerImplementor
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$LoggerImplementorCopyWith<LoggerImplementor> get copyWith => _$LoggerImplementorCopyWithImpl<LoggerImplementor>(this as LoggerImplementor, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is LoggerImplementor&&(identical(other.field0, field0) || other.field0 == field0));
}


@override
int get hashCode => Object.hash(runtimeType,field0);

@override
String toString() {
  return 'LoggerImplementor(field0: $field0)';
}


}

/// @nodoc
abstract mixin class $LoggerImplementorCopyWith<$Res>  {
  factory $LoggerImplementorCopyWith(LoggerImplementor value, $Res Function(LoggerImplementor) _then) = _$LoggerImplementorCopyWithImpl;
@useResult
$Res call({
 BindingLogger field0
});




}
/// @nodoc
class _$LoggerImplementorCopyWithImpl<$Res>
    implements $LoggerImplementorCopyWith<$Res> {
  _$LoggerImplementorCopyWithImpl(this._self, this._then);

  final LoggerImplementor _self;
  final $Res Function(LoggerImplementor) _then;

/// Create a copy of LoggerImplementor
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') @override $Res call({Object? field0 = null,}) {
  return _then(_self.copyWith(
field0: null == field0 ? _self.field0 : field0 // ignore: cast_nullable_to_non_nullable
as BindingLogger,
  ));
}

}


/// Adds pattern-matching-related methods to [LoggerImplementor].
extension LoggerImplementorPatterns on LoggerImplementor {
/// A variant of `map` that fallback to returning `orElse`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( LoggerImplementor_Variant0 value)?  variant0,required TResult orElse(),}){
final _that = this;
switch (_that) {
case LoggerImplementor_Variant0() when variant0 != null:
return variant0(_that);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// Callbacks receives the raw object, upcasted.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case final Subclass2 value:
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( LoggerImplementor_Variant0 value)  variant0,}){
final _that = this;
switch (_that) {
case LoggerImplementor_Variant0():
return variant0(_that);}
}
/// A variant of `map` that fallback to returning `null`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( LoggerImplementor_Variant0 value)?  variant0,}){
final _that = this;
switch (_that) {
case LoggerImplementor_Variant0() when variant0 != null:
return variant0(_that);case _:
  return null;

}
}
/// A variant of `when` that fallback to an `orElse` callback.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( BindingLogger field0)?  variant0,required TResult orElse(),}) {final _that = this;
switch (_that) {
case LoggerImplementor_Variant0() when variant0 != null:
return variant0(_that.field0);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// As opposed to `map`, this offers destructuring.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case Subclass2(:final field2):
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( BindingLogger field0)  variant0,}) {final _that = this;
switch (_that) {
case LoggerImplementor_Variant0():
return variant0(_that.field0);}
}
/// A variant of `when` that fallback to returning `null`
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( BindingLogger field0)?  variant0,}) {final _that = this;
switch (_that) {
case LoggerImplementor_Variant0() when variant0 != null:
return variant0(_that.field0);case _:
  return null;

}
}

}

/// @nodoc


class LoggerImplementor_Variant0 extends LoggerImplementor {
  const LoggerImplementor_Variant0(this.field0): super._();
  

@override final  BindingLogger field0;

/// Create a copy of LoggerImplementor
/// with the given fields replaced by the non-null parameter values.
@override @JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$LoggerImplementor_Variant0CopyWith<LoggerImplementor_Variant0> get copyWith => _$LoggerImplementor_Variant0CopyWithImpl<LoggerImplementor_Variant0>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is LoggerImplementor_Variant0&&(identical(other.field0, field0) || other.field0 == field0));
}


@override
int get hashCode => Object.hash(runtimeType,field0);

@override
String toString() {
  return 'LoggerImplementor.variant0(field0: $field0)';
}


}

/// @nodoc
abstract mixin class $LoggerImplementor_Variant0CopyWith<$Res> implements $LoggerImplementorCopyWith<$Res> {
  factory $LoggerImplementor_Variant0CopyWith(LoggerImplementor_Variant0 value, $Res Function(LoggerImplementor_Variant0) _then) = _$LoggerImplementor_Variant0CopyWithImpl;
@override @useResult
$Res call({
 BindingLogger field0
});




}
/// @nodoc
class _$LoggerImplementor_Variant0CopyWithImpl<$Res>
    implements $LoggerImplementor_Variant0CopyWith<$Res> {
  _$LoggerImplementor_Variant0CopyWithImpl(this._self, this._then);

  final LoggerImplementor_Variant0 _self;
  final $Res Function(LoggerImplementor_Variant0) _then;

/// Create a copy of LoggerImplementor
/// with the given fields replaced by the non-null parameter values.
@override @pragma('vm:prefer-inline') $Res call({Object? field0 = null,}) {
  return _then(LoggerImplementor_Variant0(
null == field0 ? _self.field0 : field0 // ignore: cast_nullable_to_non_nullable
as BindingLogger,
  ));
}


}

/// @nodoc
mixin _$RestClientImplementor {

 ReqwestRestClient get field0;
/// Create a copy of RestClientImplementor
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$RestClientImplementorCopyWith<RestClientImplementor> get copyWith => _$RestClientImplementorCopyWithImpl<RestClientImplementor>(this as RestClientImplementor, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is RestClientImplementor&&(identical(other.field0, field0) || other.field0 == field0));
}


@override
int get hashCode => Object.hash(runtimeType,field0);

@override
String toString() {
  return 'RestClientImplementor(field0: $field0)';
}


}

/// @nodoc
abstract mixin class $RestClientImplementorCopyWith<$Res>  {
  factory $RestClientImplementorCopyWith(RestClientImplementor value, $Res Function(RestClientImplementor) _then) = _$RestClientImplementorCopyWithImpl;
@useResult
$Res call({
 ReqwestRestClient field0
});




}
/// @nodoc
class _$RestClientImplementorCopyWithImpl<$Res>
    implements $RestClientImplementorCopyWith<$Res> {
  _$RestClientImplementorCopyWithImpl(this._self, this._then);

  final RestClientImplementor _self;
  final $Res Function(RestClientImplementor) _then;

/// Create a copy of RestClientImplementor
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') @override $Res call({Object? field0 = null,}) {
  return _then(_self.copyWith(
field0: null == field0 ? _self.field0 : field0 // ignore: cast_nullable_to_non_nullable
as ReqwestRestClient,
  ));
}

}


/// Adds pattern-matching-related methods to [RestClientImplementor].
extension RestClientImplementorPatterns on RestClientImplementor {
/// A variant of `map` that fallback to returning `orElse`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( RestClientImplementor_Variant0 value)?  variant0,required TResult orElse(),}){
final _that = this;
switch (_that) {
case RestClientImplementor_Variant0() when variant0 != null:
return variant0(_that);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// Callbacks receives the raw object, upcasted.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case final Subclass2 value:
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( RestClientImplementor_Variant0 value)  variant0,}){
final _that = this;
switch (_that) {
case RestClientImplementor_Variant0():
return variant0(_that);}
}
/// A variant of `map` that fallback to returning `null`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( RestClientImplementor_Variant0 value)?  variant0,}){
final _that = this;
switch (_that) {
case RestClientImplementor_Variant0() when variant0 != null:
return variant0(_that);case _:
  return null;

}
}
/// A variant of `when` that fallback to an `orElse` callback.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( ReqwestRestClient field0)?  variant0,required TResult orElse(),}) {final _that = this;
switch (_that) {
case RestClientImplementor_Variant0() when variant0 != null:
return variant0(_that.field0);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// As opposed to `map`, this offers destructuring.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case Subclass2(:final field2):
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( ReqwestRestClient field0)  variant0,}) {final _that = this;
switch (_that) {
case RestClientImplementor_Variant0():
return variant0(_that.field0);}
}
/// A variant of `when` that fallback to returning `null`
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( ReqwestRestClient field0)?  variant0,}) {final _that = this;
switch (_that) {
case RestClientImplementor_Variant0() when variant0 != null:
return variant0(_that.field0);case _:
  return null;

}
}

}

/// @nodoc


class RestClientImplementor_Variant0 extends RestClientImplementor {
  const RestClientImplementor_Variant0(this.field0): super._();
  

@override final  ReqwestRestClient field0;

/// Create a copy of RestClientImplementor
/// with the given fields replaced by the non-null parameter values.
@override @JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$RestClientImplementor_Variant0CopyWith<RestClientImplementor_Variant0> get copyWith => _$RestClientImplementor_Variant0CopyWithImpl<RestClientImplementor_Variant0>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is RestClientImplementor_Variant0&&(identical(other.field0, field0) || other.field0 == field0));
}


@override
int get hashCode => Object.hash(runtimeType,field0);

@override
String toString() {
  return 'RestClientImplementor.variant0(field0: $field0)';
}


}

/// @nodoc
abstract mixin class $RestClientImplementor_Variant0CopyWith<$Res> implements $RestClientImplementorCopyWith<$Res> {
  factory $RestClientImplementor_Variant0CopyWith(RestClientImplementor_Variant0 value, $Res Function(RestClientImplementor_Variant0) _then) = _$RestClientImplementor_Variant0CopyWithImpl;
@override @useResult
$Res call({
 ReqwestRestClient field0
});




}
/// @nodoc
class _$RestClientImplementor_Variant0CopyWithImpl<$Res>
    implements $RestClientImplementor_Variant0CopyWith<$Res> {
  _$RestClientImplementor_Variant0CopyWithImpl(this._self, this._then);

  final RestClientImplementor_Variant0 _self;
  final $Res Function(RestClientImplementor_Variant0) _then;

/// Create a copy of RestClientImplementor
/// with the given fields replaced by the non-null parameter values.
@override @pragma('vm:prefer-inline') $Res call({Object? field0 = null,}) {
  return _then(RestClientImplementor_Variant0(
null == field0 ? _self.field0 : field0 // ignore: cast_nullable_to_non_nullable
as ReqwestRestClient,
  ));
}


}

// dart format on
